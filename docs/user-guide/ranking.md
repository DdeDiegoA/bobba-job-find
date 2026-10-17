# Ranking

Every Offer gets two scores, one honest verdict, and a plain-language reason.

## Two scores, one rank

- **Fit Score**: can you get it? Skills, experience, location, language, education, and,
  if you turn it on, a semantic match.
- **Quality Score**: do you want it? Pay against the market, company signals, role and terms,
  legitimacy, and how well it fits where you're going.
- **Rank Score**: the two combined. It decides the **Verdict**: **Strong** (75+), **Good**
  (60-74), **Moderate** (45-59), **Weak** or **Poor**.

Scoring is deterministic and runs on your computer. The same Offer and the same Profile give
the same score, and the AI only proposes extracted details that the core then checks.

## Gates

Some of your preferences are hard limits, such as a salary floor set to "hard". An Offer that
trips one is **Excluded** and Boba keeps the reason and the exact quote on record. If you
disagree, **Override veto** brings it back; Boba notes that you checked it.

## Reading an Offer

Open an Offer to see its score breakdown and the **Explanation**: where it's strong, where
it's a reach, and a next step. A "reach" is not a "no"; you decide. Offers the AI hasn't
analysed yet are still ranked from the posting's own words and marked **Low confidence**.

## Badges

Boba flags things worth a second look: an Offer that asks for a fee, off-platform contact,
implausible pay, a stale posting, a vague description, no salary, or talent-pool wording.
Each badge shows where it came from. A fraud badge also stops Auto from sending that Offer.

## Tuning

**Settings → Scoring** lets you change how much each factor counts and how Fit and Quality
combine. Every change creates a new version, and your open Offers are re-scored with it.
**Reset to defaults** is always there.

**Settings → AI engine** sets how many Offers one run hands to the AI (default 30); the rest
are ranked from the posting text alone. It also shows the per-task limits and what your CLI
reports about token use.

## Triage

**Today** shows your closest matches. **Inbox** is where you work through them:

- **Save** puts an Offer in your Tracker. **Dismiss** removes it, with an optional reason that
  teaches Boba what you don't want.
- Select several to dismiss in bulk, and undo any move.
- Keyboard shortcuts and a command palette keep triage quick.

Next: [Studio](studio.md).
