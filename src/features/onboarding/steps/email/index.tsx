import { openUrl } from "@tauri-apps/plugin-opener";
import { useId, useState } from "react";
import { useTranslation } from "react-i18next";
import { Link } from "react-router";
import { Button } from "@/components/ui/button";
import {
  EMAIL_OWN_CLIENT_GUIDE_URL,
  useBundledEmailClients,
  useConnectGmail,
  useConnectOutlook,
  useEmailAccounts,
} from "@/features/connections/email/use-email";
import { isAppError, renderAppError } from "@/lib/ipc/errors";
import type { StepProps } from "../../wizard/machine";

/** How a connection's provider reads (AD-21: resolvable keys). */
const PROVIDER_LABELS: Record<string, string> = {
  microsoft_graph: "onboarding_finish:email.provider.microsoft_graph",
  gmail: "onboarding_finish:email.provider.gmail",
  imap_smtp: "onboarding_finish:email.provider.imap_smtp",
};

/**
 * Step — Email (FR-9). Optional: the wizard continues whether the user connects
 * a mailbox here or skips the step. The two one-click flows are the same
 * commands the Connections screen uses; the advanced forms (IMAP/SMTP, a BYO
 * client id) stay there (AC2).
 */
export default function EmailStep(_props: StepProps) {
  const { t } = useTranslation("onboarding_finish");
  const { t: tError } = useTranslation();
  const accounts = useEmailAccounts();
  const bundled = useBundledEmailClients();
  const outlook = useConnectOutlook();
  const gmail = useConnectGmail();
  const [address, setAddress] = useState("");
  const addressHintId = useId();

  // FX5b: a provider this build has no client for can never complete the
  // one-click flow, so its button is hidden and "add your own client" becomes
  // the next action instead of an error after the click. While the probe is
  // loading (or fails) the step keeps its old behaviour.
  const noOutlook = bundled.data?.outlook === false;
  const noGmail = bundled.data?.gmail === false;
  const missingProviders = [
    ...(noOutlook ? [t("email.provider.microsoft_graph")] : []),
    ...(noGmail ? [t("email.provider.gmail")] : []),
  ];

  const failure = outlook.error ?? gmail.error ?? null;
  // FX5: a build with no bundled client can never do one-click sign-in. That is
  // not an error to repeat, it is a next action: skip, or bring your own client.
  const noBundledClient =
    failure !== null &&
    isAppError(failure) &&
    failure.message_key === "errors.email.no_bundled_client";
  const errorText =
    failure && !noBundledClient
      ? isAppError(failure)
        ? renderAppError(failure, tError)
        : t("email.error")
      : null;
  const pending = outlook.isPending || gmail.isPending;
  const addressEmpty = address.trim() === "";
  const canConnect = !addressEmpty && !pending;

  return (
    <div className="flex flex-col gap-6">
      <p className="text-fg-muted">{t("email.body")}</p>

      <section className="flex flex-col gap-2">
        <h3 className="font-medium text-fg">{t("email.connected.title")}</h3>
        {accounts.isPending ? (
          <p className="text-sm text-fg-muted">{t("email.connected.loading")}</p>
        ) : null}
        {accounts.data && accounts.data.length === 0 ? (
          <p className="text-sm text-fg-muted">{t("email.connected.none")}</p>
        ) : null}
        <ul className="flex flex-col gap-1">
          {(accounts.data ?? []).map((account) => (
            <li key={account.id} className="flex flex-wrap items-center gap-2 text-sm">
              <span className="text-fg">{account.address}</span>
              <span className="rounded-xl bg-surface-muted px-2 py-1 text-xs text-fg-muted">
                {t(PROVIDER_LABELS[account.provider] ?? account.provider)}
              </span>
            </li>
          ))}
        </ul>
      </section>

      {/* No one-click provider at all: the address has nothing to feed. */}
      {missingProviders.length === 2 ? null : (
        <>
          <label className="flex flex-col gap-1 text-sm text-fg">
            {t("email.addressLabel")}
            <input
              type="email"
              className="h-10 rounded-xl border border-border bg-surface px-3 text-fg"
              placeholder={t("email.addressPlaceholder")}
              value={address}
              onChange={(event) => setAddress(event.target.value)}
            />
          </label>
          {addressEmpty ? (
            <p id={addressHintId} className="text-xs text-fg-subtle">
              {t("email.addressHint")}
            </p>
          ) : null}
        </>
      )}

      <div className="flex flex-wrap gap-2">
        {noOutlook ? null : (
          <Button
            type="button"
            variant="outline"
            disabled={!canConnect}
            aria-describedby={addressEmpty ? addressHintId : undefined}
            onClick={() =>
              outlook.mutate({
                address: address.trim(),
                mode: "send_only",
                client_id: null,
              })
            }
          >
            {outlook.isPending ? t("email.connecting") : t("email.action.outlook")}
          </Button>
        )}
        {noGmail ? null : (
          <Button
            type="button"
            variant="outline"
            disabled={!canConnect}
            aria-describedby={addressEmpty ? addressHintId : undefined}
            onClick={() =>
              gmail.mutate({
                address: address.trim(),
                client_id: null,
                client_secret: null,
              })
            }
          >
            {gmail.isPending ? t("email.connecting") : t("email.action.gmail")}
          </Button>
        )}
      </div>

      {missingProviders.length > 0 ? (
        <div className="flex flex-col gap-2 rounded-xl border border-border bg-surface-muted p-3 text-sm text-fg">
          <p className="font-medium">{t("email.addYourOwn.title")}</p>
          <p>{t("email.addYourOwn.body", { providers: missingProviders.join(", ") })}</p>
          <div className="flex flex-wrap items-center gap-3">
            <Link to="/connections" className="text-fg underline underline-offset-2">
              {t("email.addYourOwn.action")}
            </Link>
            <button
              type="button"
              className="text-fg underline underline-offset-2"
              onClick={() => void openUrl(EMAIL_OWN_CLIENT_GUIDE_URL).catch(() => undefined)}
            >
              {t("email.addYourOwn.guide")}
            </button>
          </div>
          <p className="text-xs text-fg-subtle">{t("email.addYourOwn.skip")}</p>
        </div>
      ) : null}

      {pending ? (
        <p role="status" className="text-sm text-fg-muted">
          {t("email.waiting")}
        </p>
      ) : null}

      <p className="text-xs text-fg-subtle">{t("email.note")}</p>

      {noBundledClient ? (
        <div
          role="alert"
          className="flex flex-col gap-2 rounded-xl border border-border bg-surface-muted p-3 text-sm text-fg"
        >
          <p>{t("email.noBundledClient.body")}</p>
          <Link to="/connections" className="self-start text-fg underline-offset-2 hover:underline">
            {t("email.noBundledClient.action")}
          </Link>
        </div>
      ) : null}

      {errorText ? (
        <p role="alert" className="text-sm text-danger">
          {errorText}
        </p>
      ) : null}
    </div>
  );
}
