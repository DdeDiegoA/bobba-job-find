//! Network activity and FX (S10.4, NFR-1, FR-115).
//!
//! The view itself is [`boba_app::network_log`]; these commands only gather the
//! destinations the app is *configured* to contact (a connected mailbox, a saved
//! custom feed, the opted-in model download) and add them to the log the request
//! paths already write to. Nothing here records a path, a query, a header or a
//! payload: [`boba_app::network_log::record`] keeps the host and drops the rest.
//!
//! FX is the one outbound call this slice makes itself, and only when the user
//! turns it on (AC2, NFR-1 e). It goes through the one shared `HttpClient`, so
//! it is paced, capped and recorded like every other request.

use boba_app::embeddings::EmbeddingsService;
use boba_app::error::{AppError, AppResult};
use boba_app::network_log::fx::{self, FxError, FxSettings};
use boba_app::network_log::{
    self, Declared, MODEL_HOST, NetworkActivity, NetworkLogError, Purpose, UPDATE_CHECK_KEY,
    UPDATE_HOST,
};
use boba_app::ports::repo::email::{EmailRepo, MailProvider};
use boba_app::ports::repo::system::SystemRepo;
use boba_domain::source::RateLimits;
use boba_sources::http::{FetchKind, FetchRequest, HttpClient};
use tauri::AppHandle;

use crate::wiring;

/// The `setting` row that turns update checks on (S10.5 writes it).
const UPDATE_ON: &str = "on";

/// Settings → Privacy → Network activity (AC1): every destination contacted
/// this session, plus the ones the current configuration will contact, grouped
/// by purpose. There is no telemetry group: the model has none.
#[tauri::command]
#[specta::specta]
pub fn network_activity(app: AppHandle) -> AppResult<NetworkActivity> {
    let mut declared = Vec::new();

    // A connected mailbox is a destination the app will contact, even before
    // the next sync. IMAP/SMTP hosts are recorded when a socket opens, so only
    // the two OAuth providers have a host to declare here.
    if let Ok(email) = wiring::port::<dyn EmailRepo>(&app) {
        for account in email.list_accounts().unwrap_or_default() {
            let Some(host) = provider_host(account.provider) else {
                continue;
            };
            // The mailbox address is the user's personal data (NFR-2: logs
            // redact emails), so the row carries no account detail at all.
            declared.push(Declared::new(Purpose::Email, host).map_err(declared_error)?);
        }
    }

    // Saved custom feeds are the sources whose hosts the app knows without a
    // request; the built-in adapters appear once a run fetches from them.
    let system = wiring::port::<dyn SystemRepo>(&app)?;
    if let Ok(feeds) = boba_app::sources::custom::list_custom_feeds(system.as_ref()) {
        for feed in feeds {
            let Some(host) = host_label(&feed.url) else {
                continue;
            };
            declared.push(
                Declared::new(Purpose::Sources, &host)
                    .map_err(declared_error)?
                    .with_note(&feed.name),
            );
        }
    }

    // Update checks are opt-in and off unless S10.5's switch says otherwise.
    if system
        .setting_get(UPDATE_CHECK_KEY)
        .ok()
        .flatten()
        .is_some_and(|value| value.trim() == UPDATE_ON)
    {
        declared.push(Declared::new(Purpose::Update, UPDATE_HOST).map_err(declared_error)?);
    }

    // The optional model download (S6.9) already recorded when it finished, so
    // it enters the log as an observed contact with a real time rather than as
    // a configured row.
    if let Ok(embeddings) = wiring::port::<EmbeddingsService>(&app) {
        for entry in embeddings.network_activity().unwrap_or_default() {
            if entry.downloaded_at.trim().is_empty() {
                continue;
            }
            network_log::observe(
                Purpose::ModelDownload,
                MODEL_HOST,
                &entry.downloaded_at,
                Some(&entry.model_id),
            );
        }
    }

    // FX is off until the user says otherwise; once on, its host is a
    // destination the app will contact.
    let fx_settings = fx::load(system.as_ref()).map_err(fx_error)?;
    if fx_settings.enabled {
        declared.push(Declared::new(Purpose::Fx, fx::SOURCE_HOST).map_err(declared_error)?);
    }

    Ok(network_log::snapshot(&declared))
}

/// The FX opt-in and the last rates fetched (AC2).
#[tauri::command]
#[specta::specta]
pub fn fx_settings(app: AppHandle) -> AppResult<FxSettings> {
    let system = wiring::port::<dyn SystemRepo>(&app)?;
    fx::load(system.as_ref()).map_err(fx_error)
}

/// Turn salary conversion on (fetching today's rates once) or off.
///
/// Enabling fetches the rates for the display currency before the setting is
/// stored: a failed fetch leaves FX off, so the UI can never say a conversion is
/// available when there is no rate behind it.
#[tauri::command]
#[specta::specta]
pub async fn fx_set_enabled(app: AppHandle, enabled: bool) -> AppResult<FxSettings> {
    let system = wiring::port::<dyn SystemRepo>(&app)?;
    let mut settings = fx::load(system.as_ref()).map_err(fx_error)?;

    if !enabled {
        settings.enabled = false;
        fx::save(system.as_ref(), &settings, &now()).map_err(fx_error)?;
        return Ok(settings);
    }

    let stored_target = settings.target.trim().to_ascii_uppercase();
    let target = if stored_target.is_empty() {
        display_currency(system.as_ref())
    } else {
        stored_target
    };

    let http = wiring::port::<HttpClient>(&app)?;
    let url = fx::source_url(&target);
    let response = http
        .get_for(
            Purpose::Fx,
            FetchRequest {
                url: &url,
                kind: FetchKind::Json,
                limits: RateLimits::default(),
                budget_key: None,
            },
        )
        .await
        .map_err(|_| AppError::internal("errors.network.fx_unreachable"))?;
    if response.status != 200 {
        return Err(AppError::internal("errors.network.fx_unreachable"));
    }
    let quote = fx::parse_latest(&response.text(), &target).map_err(fx_error)?;

    settings.enabled = true;
    settings.target = target;
    settings.as_of = Some(quote.as_of);
    settings.rates = quote.rates;
    fx::save(system.as_ref(), &settings, &now()).map_err(fx_error)?;
    Ok(settings)
}

/// The host (`graph.microsoft.com`) a provider's traffic goes to; `None` for
/// IMAP/SMTP, whose user-typed host is recorded when the socket opens.
fn provider_host(provider: MailProvider) -> Option<&'static str> {
    match provider {
        MailProvider::MicrosoftGraph => Some("graph.microsoft.com"),
        MailProvider::Gmail => Some("gmail.googleapis.com"),
        MailProvider::ImapSmtp => None,
    }
}

/// The host of a saved feed URL, `None` when it is not an https URL (a feed
/// the runner will refuse anyway).
fn host_label(url: &str) -> Option<String> {
    network_log::host_of(url).ok()
}

/// The currency conversions are made *to*: the Profile's salary floor currency
/// (FR-7), else [`fx::DEFAULT_TARGET`]. Read as JSON so this slice never owns
/// the preferences document's shape.
fn display_currency(system: &dyn SystemRepo) -> String {
    system
        .setting_get(crate::commands::preferences::KEY)
        .ok()
        .flatten()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .and_then(|document| {
            document
                .get("salary_floor")
                .and_then(|floor| floor.get("currency"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        })
        .filter(|currency| !currency.trim().is_empty())
        .unwrap_or_else(|| fx::DEFAULT_TARGET.to_string())
}

fn now() -> String {
    use boba_app::ports::clock::ClockPort as _;
    boba_app::system::SystemClock.now_rfc3339()
}

fn declared_error(error: NetworkLogError) -> AppError {
    AppError::internal("errors.network.invalid_destination")
        .with_detail("reason", error.to_string())
}

fn fx_error(error: FxError) -> AppError {
    AppError::internal(error.message_key())
}
