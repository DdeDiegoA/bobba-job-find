-- 0002_cli_install.sql — detected agent CLI installs (S2.1). Never edit once shipped (AD-20).
--
-- Owner: agent (AD-16). One row per probed CLI installation; `(kind, path)` is
-- unique so a re-probe refreshes the row in place. `capabilities_json` holds the
-- probed capability flags (AD-4). `probed_at` is UTC RFC 3339 (AD-17).
CREATE TABLE cli_install (
    id                TEXT PRIMARY KEY NOT NULL,
    kind              TEXT NOT NULL,
    path              TEXT NOT NULL,
    version           TEXT NOT NULL,
    capabilities_json TEXT NOT NULL,
    probed_at         TEXT NOT NULL,
    UNIQUE (kind, path)
) STRICT;
