-- 0014_document_review_json.sql — the fresh-context reviewer's critique (S7.4,
-- FR-22/FR-72). Never edit once shipped (AD-20).
--
-- `save_review` stores the review run's findings here, on the drafts it judged,
-- so the studio can show them again after a reload. `review_json` is the
-- findings array as text and is never evaluated: the studio renders every field
-- as data (ADR-010). NULL is the normal case — no review ran.
--
-- The critique carries its own `untrusted_origin` marker rather than relying on
-- the row's: it is agent text even when the user later edits the draft it sits
-- on, and the studio must render it as data either way. Default 1 stamps every
-- row that wrote a critique before this marker existed (there are none) and
-- every critique a review run writes; a user's own words are the only thing that
-- writes 0.
ALTER TABLE document ADD COLUMN review_json TEXT;
ALTER TABLE document ADD COLUMN review_untrusted_origin INTEGER NOT NULL DEFAULT 1
    CHECK (review_untrusted_origin IN (0, 1));
