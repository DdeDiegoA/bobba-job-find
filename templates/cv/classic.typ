// templates/cv/classic.typ — ATS-safe single-column CV (S7.1, AD-14).
//
// The CvDocument JSON arrives in sys.inputs.data. Every string is printed as
// text and never evaluated as Typst markup: agent output is untrusted (ADR-010).
// Standard headings, one column, no tables, no images, contact details literal.

#let doc = json(sys.inputs.data)
#let contact = doc.contact
#let field(d, key) = if key in d { d.at(key) } else { "" }
#let lines(values) = values.filter(v => v != "").join("  |  ")

#set page(
  paper: if doc.lang == "es" { "a4" } else { "us-letter" },
  margin: (x: 0.7in, y: 0.7in),
)
#set text(font: "Lato", size: 10pt, lang: doc.lang, fill: rgb("#181818"))
#set par(justify: false, leading: 0.6em, spacing: 0.8em)

#let sec(title) = block(above: 1em, below: 0.4em)[
  #text(size: 11pt, weight: "bold")[#title]
  #v(-0.15em)
  #line(length: 100%, stroke: 0.6pt + rgb("#444444"))
]

#text(size: 19pt, weight: "bold")[#contact.full_name]
#if "headline" in contact [
  #linebreak()
  #text(size: 10.5pt, fill: rgb("#444444"))[#contact.headline]
]
#linebreak()
#text(size: 9pt)[#lines((field(contact, "email"), field(contact, "phone"), field(contact, "location")))]
#if contact.at("links", default: ()).len() > 0 [
  #linebreak()
  #text(size: 9pt)[#contact.links.map(l => l.label + ": " + l.url).join("  |  ")]
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
