-- 0001_init.sql — base metadata (S1.2a). Never edit once shipped (AD-20).
--
-- Only the generic key/value meta table lives here; every later slice owns its
-- own NNNN_*.sql. The migration runner discovers files by sorted file name.
CREATE TABLE boba_meta (
    key   TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
) STRICT;
