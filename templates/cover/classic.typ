// templates/cover/classic.typ — ATS-safe one-page cover letter (S7.1, AD-14).
//
// The CoverLetterDocument JSON arrives in sys.inputs.data. Strings are printed
// as text, never evaluated as Typst markup (ADR-010).

#let doc = json(sys.inputs.data)
#let contact = doc.contact
#let field(d, key) = if key in d { d.at(key) } else { "" }
#let lines(values) = values.filter(v => v != "").join("  |  ")

#set page(
  paper: if doc.lang == "es" { "a4" } else { "us-letter" },
  margin: 1in,
)
#set text(font: "Lato", size: 11pt, lang: doc.lang, fill: rgb("#181818"))
#set par(justify: false, leading: 0.7em, spacing: 1em)

#text(size: 16pt, weight: "bold")[#contact.full_name]
#linebreak()
#text(size: 9.5pt)[#lines((field(contact, "email"), field(contact, "phone"), field(contact, "location")))]

#v(1.2em)
#field(doc, "date")
#if "company" in doc [
  #linebreak()
  #text(weight: "bold")[#doc.company]
]
#if "recipient" in doc [
  #linebreak()
  #doc.recipient
]

#v(1em)
#if "salutation" in doc [
  #doc.salutation
  #parbreak()
]
#for paragraph in doc.at("paragraphs", default: ()) [
  #paragraph.text
  #parbreak()
]
#if "closing" in doc [#doc.closing]
