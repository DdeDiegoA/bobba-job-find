import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { commands, type EmbeddingStatus } from "@/lib/ipc/bindings";
import { isAppError, renderAppError, unwrap } from "@/lib/ipc/errors";

/**
 * Settings → Scoring → On-device semantic match (S6.9, FR-55, AD-8).
 *
 * F3 is the one factor Boba cannot compute from text matching alone: it needs
 * a small language model on disk. The model is a **choice**, not a default —
 * this card states what the download costs (~487 MB), shows where it goes, and
 * only fetches it after the user turns it on. A deterministic build stays the
 * default; without the model the factor is skipped and its weight spread over
 * the others (AC2).
 */
export default function SemanticMatch() {
  const { t } = useTranslation("settings_scoring");
  const { t: tError } = useTranslation();
  const queryClient = useQueryClient();
  const [error, setError] = useState<string | null>(null);

  const status = useQuery({
    queryKey: ["embeddings"],
    queryFn: () => commands.embeddingsStatus().then(unwrap),
  });

  const showError = (caught: unknown) =>
    setError(isAppError(caught) ? renderAppError(caught, tError) : t("embeddings.error"));

  const turnOn = useMutation({
    mutationFn: () => commands.embeddingsOptIn().then(unwrap),
    onSuccess: (data: EmbeddingStatus) => {
      queryClient.setQueryData(["embeddings"], data);
      setError(null);
    },
    onError: showError,
  });

  const turnOff = useMutation({
    mutationFn: async () => {
      await commands.embeddingsOptOut().then(unwrap);
      return commands.embeddingsStatus().then(unwrap);
    },
    onSuccess: (data: EmbeddingStatus) => {
      queryClient.setQueryData(["embeddings"], data);
      setError(null);
    },
    onError: showError,
  });

  const value = status.data;
  const busy = turnOn.isPending || turnOff.isPending;
  const megabytes = value ? Math.round(value.approximate_bytes / (1024 * 1024)) : 0;

  return (
    <fieldset className="flex flex-col gap-2" aria-labelledby="scoring-embeddings-title">
      <legend id="scoring-embeddings-title" className="font-display text-lg text-fg">
        {t("embeddings.title")}
      </legend>
      <p className="text-sm text-fg-muted">{t("embeddings.hint", { size: megabytes })}</p>

      {status.isPending ? <p className="text-sm text-fg-muted">{t("embeddings.loading")}</p> : null}
      {status.isError ? (
        <p className="text-sm text-danger">
          {isAppError(status.error) ? renderAppError(status.error, tError) : t("embeddings.error")}
        </p>
      ) : null}

      {value && !value.available ? (
        <p role="status" className="text-sm text-fg-muted">
          {t("embeddings.unavailable")}
        </p>
      ) : null}

      {value?.available ? (
        <div className="flex flex-wrap items-center gap-3">
          {value.opted_in ? (
            <>
              <p role="status" className="text-sm text-fg">
                {t("embeddings.on")}
              </p>
              <Button
                type="button"
                variant="outline"
                disabled={busy}
                onClick={() => turnOff.mutate()}
              >
                {t("embeddings.off")}
              </Button>
            </>
          ) : (
            <Button type="button" disabled={busy} onClick={() => turnOn.mutate()}>
              {t("embeddings.enable")}
            </Button>
          )}
        </div>
      ) : null}

      {busy ? (
        <p role="status" className="text-sm text-fg-muted">
          {t("embeddings.downloading")}
        </p>
      ) : null}

      {value?.opted_in && value.destination ? (
        <p className="text-sm text-fg-muted">
          {t("embeddings.destination", { path: value.destination })}
        </p>
      ) : null}

      {error ? (
        <p role="alert" className="text-sm text-danger">
          {error}
        </p>
      ) : null}
    </fieldset>
  );
}
