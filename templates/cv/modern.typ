// templates/cv/modern.typ — ATS-safe single-column CV, accent theme (S7.1, AD-14).
//
// Same content as classic.typ with a warmer, modern look. Strings are data only
// (ADR-010). One column, standard headings, no tables or images.

#let doc = json(sys.inputs.data)
#let contact = doc.contact
#let field(d, key) = if key in d { d.at(key) } else { "" }
#let lines(values) = values.filter(v => v != "").join("   •   ")
#let accent = rgb("#0f766e")

#set page(
  paper: if doc.lang == "es" { "a4" } else { "us-letter" },
  margin: (x: 0.75in, y: 0.7in),
)
#set text(font: "Lato", size: 10pt, lang: doc.lang, fill: rgb("#1f2933"))
#set par(justify: false, leading: 0.6em, spacing: 0.8em)

#let sec(title) = block(above: 1em, below: 0.45em)[
  #text(size: 11pt, weight: "bold", fill: accent, tracking: 0.4pt)[#title]
  #v(-0.2em)
  #line(length: 1.1in, stroke: 1.4pt + accent)
]

#text(size: 22pt, weight: "bold")[#contact.full_name]
#if "headline" in contact [
  #linebreak()
  #text(size: 10.5pt, fill: rgb("#52606d"))[#contact.headline]
]
#linebreak()
#text(size: 9pt)[#lines((field(contact, "email"), field(contact, "phone"), field(contact, "location")))]
#if contact.at("links", default: ()).len() > 0 [
  #linebreak()
  #text(size: 9pt)[#contact.links.map(l => l.label + ": " + l.url).join("   •   ")]
]

#if "summary" in doc [
  #sec("Summary")
  #doc.summary.text
]

#for section in doc.at("sections", default: ()) [
  #sec(section.heading)
  #for item in section.items [
    - #item.text
  ]
]
