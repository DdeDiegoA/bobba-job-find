//! `ApplyRepo` — owns `application`, `apply_attempt`, `status_event`, `outbox`,
//! `application_answer_bank` (AD-12, AD-16). Status changes go only through the
//! apply `TransitionStatus` use case.
