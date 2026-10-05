-- 0003_documents.sql — tailored documents (S7.1, AD-14). Never edit once shipped (AD-20).
--
-- One row per rendered/generated document. `model_json` is the schema-versioned
-- CvDocument / CoverLetterDocument (AD-14); blobs are the rendered PDF/DOCX and
-- stay NULL until rendered. `verification_json` / `verified_at` are filled by the
-- verifier slices (S7.5/S7.6); only a verified document may be submitted.
--
-- The owner is boba-store/src/repos/documents.rs (AD-16). `application_id` is a
-- plain ULID reference; the `application` table is owned by apply and is created
-- in a later migration, so no foreign key is declared yet.
CREATE TABLE document (
    id                TEXT    PRIMARY KEY NOT NULL,
    application_id    TEXT    NOT NULL,
    kind              TEXT    NOT NULL CHECK (kind IN ('cv', 'cover_letter')),
    lang              TEXT    NOT NULL CHECK (lang IN ('en', 'es')),
    template          TEXT    NOT NULL,
    version           INTEGER NOT NULL DEFAULT 1,
    model_json        TEXT    NOT NULL,
    pdf_blob          BLOB,
    docx_blob         BLOB,
    verification_json TEXT,
    verified_at       TEXT,
    created_at        TEXT    NOT NULL,
    updated_at        TEXT    NOT NULL
) STRICT;

-- One version per (application, kind): regenerating a document bumps `version`.
CREATE UNIQUE INDEX document_app_kind_version_idx
    ON document (application_id, kind, version);

CREATE INDEX document_application_idx ON document (application_id);
