-- 0012_document_untrusted_origin.sql — where a document's text came from (S7.2, AC7).
-- Never edit once shipped (AD-20).
--
-- Forward-only: 0003_documents.sql is already shipped, so the column is added
-- here rather than edited there. Drafts are written by an agent run that read
-- third-party content, so they are untrusted data (ADR-010): the default is 1,
-- which also stamps every row that existed before this migration. A user's own
-- edit is the one thing that writes 0. The UI and the Typst renderer already
-- treat the column as data only — nothing here evaluates it.
ALTER TABLE document ADD COLUMN untrusted_origin INTEGER NOT NULL DEFAULT 1
    CHECK (untrusted_origin IN (0, 1));
