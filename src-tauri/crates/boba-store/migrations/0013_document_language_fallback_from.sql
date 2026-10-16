-- 0013_document_language_fallback_from.sql — why a draft is not in the Offer's
-- language (S7.2, FR-70). Never edit once shipped (AD-20).
--
-- FR-70 drafts in the Offer's language when it is ES or EN, and in EN otherwise,
-- with a notice. The Offer's language is stored here so the studio can still
-- explain an EN draft of, say, a German posting after a reload, without
-- re-deriving it from an Offer that may since have been refreshed or deleted.
-- NULL is the normal case: the draft is in the Offer's own language, or in the
-- language the user overrode it to, and there is nothing to explain.
ALTER TABLE document ADD COLUMN language_fallback_from TEXT;
