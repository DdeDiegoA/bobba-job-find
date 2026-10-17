import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import {
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
  const outlook = useConnectOutlook();
  const gmail = useConnectGmail();
  const [address, setAddress] = useState("");

  const failure = outlook.error ?? gmail.error ?? null;
  const errorText = failure
    ? isAppError(failure)
      ? renderAppError(failure, tError)
      : t("email.error")
    : null;
  const pending = outlook.isPending || gmail.isPending;
  const canConnect = address.trim() !== "" && !pending;

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

      <div className="flex flex-wrap gap-2">
        <Button
          type="button"
          variant="outline"
          disabled={!canConnect}
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
        <Button
          type="button"
          variant="outline"
          disabled={!canConnect}
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
      </div>

      <p className="text-xs text-fg-subtle">{t("email.note")}</p>

      {errorText ? (
        <p role="alert" className="text-sm text-danger">
          {errorText}
        </p>
      ) : null}
    </div>
  );
}
