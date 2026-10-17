---
name: translate_profile
description: Draft the missing EN or ES variant of one Profile fact for the user to review and confirm.
---

# Draft the missing variant of one Profile fact

One fact in the user's Profile has only one language variant. You draft the
other one. The user reads it, edits it if they want, and confirms it — nothing
you write is saved before that (AD-15).

## Read first

1. `get_task_input` — the run's job: `target` (the BCP-47 tag to draft, `en` or
   `es`) and `fact` (the item's `item_id`, its `kind`, its existing variant in
   `text_en` or `text_es`, its `evidence` and its structured `detail`).
2. `get_profile_facts` — the whole Profile, for context. Use it to match the
   user's voice and the fact's own vocabulary; never to add claims this fact
   does not make.

The existing variant is the **only** source you may translate. Its wording is
the user's own; the fact's structured `detail` (dates, organisation, CEFR level)
travels with the fact and is not yours to change.

## What you do

Call `propose_translations` **once**:

```json
{"translations": [
  {"item_id": "01J8Z…",
   "lang": "es",
   "text": "Preaviso: 1 mes"}
]}
```

The hard rules:

- **Copy `item_id` exactly** as `get_task_input` gave it, and use the `target`
  tag as `lang`. A translation for another item, or into the variant that
  already exists, is dropped by the core.
- **Translate, do not improve.** Same meaning, same length order of magnitude,
  same claims. Do not add a metric, a skill, an employer, a date or a
  qualification the original variant does not state, and do not soften a level
  (`C1` stays `C1`; a level never becomes "native" unless the fact says so).
- **Plain text.** No markdown, no HTML, no quotes around the text, no notes and
  no explanation: `text` is exactly what the user will see in the field.
- Keep proper nouns (employers, institutions, product names) as they are.
- Write in the **target** language only; never mix the two variants in one
  string.

When the call is accepted, you are done. The user reviews the draft next.
