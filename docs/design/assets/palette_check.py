#!/usr/bin/env python3
"""WCAG contrast verification for the Boba Job Find design system (DESIGN.md §2.4, §3.2).

Runnable check for the one piece of the design system that can silently break: color contrast.
Run from the repo root:   python3 docs/design/assets/palette_check.py
Exits 1 on any failure, printing the exact pair and ratio.

If you change a semantic token or a score-band color in DESIGN.md, update the values here and re-run.
"""
import sys


def _srgb(c: float) -> float:
    c /= 255.0
    return c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4


def luminance(hex_: str) -> float:
    h = hex_.lstrip("#")
    r, g, b = (int(h[i:i + 2], 16) for i in (0, 2, 4))
    return 0.2126 * _srgb(r) + 0.7152 * _srgb(g) + 0.0722 * _srgb(b)


def ratio(a: str, b: str) -> float:
    la, lb = luminance(a), luminance(b)
    hi, lo = max(la, lb), min(la, lb)
    return (hi + 0.05) / (lo + 0.05)


# name: (foreground, background, required ratio)
TEXT_PAIRS = {
    # --- light semantic ---
    "light/text on bg": ("#2B201A", "#FFFBF5", 4.5),
    "light/text-muted on bg": ("#6B594C", "#FFFBF5", 4.5),
    "light/text on surface": ("#2B201A", "#FFFFFF", 4.5),
    "light/text-muted on surface": ("#6B594C", "#FFFFFF", 4.5),
    "light/on-primary vs primary": ("#FFFFFF", "#9A5418", 4.5),
    "light/link on bg": ("#8A4A16", "#FFFBF5", 4.5),
    "light/text on muted": ("#2B201A", "#F5ECE0", 4.5),
    "light/text-muted on muted": ("#6B594C", "#F5ECE0", 4.5),
    "light/danger": ("#A3281C", "#FFFFFF", 4.5),
    "light/success": ("#2E6B3E", "#FFFFFF", 4.5),
    "light/warning": ("#8A5300", "#FFFFFF", 4.5),
    "light/info": ("#5B3E8A", "#FFFFFF", 4.5),
    # --- dark semantic ---
    "dark/text on bg": ("#F5EDE4", "#191210", 4.5),
    "dark/text-muted on bg": ("#BDAB9A", "#191210", 4.5),
    "dark/text on surface": ("#F5EDE4", "#241A16", 4.5),
    "dark/text-muted on surface": ("#BDAB9A", "#241A16", 4.5),
    "dark/on-primary vs primary": ("#241A16", "#E8A860", 4.5),
    "dark/link on bg": ("#F0B878", "#191210", 4.5),
    "dark/text on muted": ("#F5EDE4", "#2E221C", 4.5),
    "dark/text-muted on muted": ("#BDAB9A", "#2E221C", 4.5),
    "dark/danger": ("#F2A49B", "#241A16", 4.5),
    "dark/success": ("#8FD3A0", "#241A16", 4.5),
    "dark/warning": ("#F0C070", "#241A16", 4.5),
    "dark/info": ("#C4A6F0", "#241A16", 4.5),
}

# band: (chip fill, on-fill text, text-on-light, text-on-dark)
BANDS = {
    "Strong   75-100": ("#2E7D46", "#FFFFFF", "#2E6B3E", "#8FD3A0"),
    "Good     60-74":  ("#5A7A12", "#FFFFFF", "#4C660C", "#B6D46A"),
    "Moderate 45-59":  ("#9E6203", "#FFFFFF", "#8A5300", "#F0C070"),
    "Weak     30-44":  ("#B84E12", "#FFFFFF", "#9A3E10", "#F0A578"),
    "Poor     0-29":   ("#A82E24", "#FFFFFF", "#A3281C", "#F2A49B"),
}


def main() -> int:
    fails = 0
    print("Text pairs (AA normal text = 4.5:1)")
    for name, (fg, bg, need) in TEXT_PAIRS.items():
        r = ratio(fg, bg)
        ok = r >= need
        fails += 0 if ok else 1
        print(f"  {'PASS' if ok else 'FAIL'}  {r:5.2f}  need {need:.1f}  {name}")

    print("\nScore bands (fill text + label text on light/dark)")
    for name, (fill, on, lt, dt) in BANDS.items():
        for label, r, need in (
            ("fill", ratio(fill, on), 4.5),
            ("label/light", ratio(lt, "#FFFFFF"), 4.5),
            ("label/dark", ratio(dt, "#241A16"), 4.5),
        ):
            ok = r >= need
            fails += 0 if ok else 1
            print(f"  {'PASS' if ok else 'FAIL'}  {r:5.2f}  need {need:.1f}  {name} {label}")

    print()
    if fails:
        print(f"{fails} FAILURE(S) — fix the tokens in DESIGN.md and here, then re-run.")
        return 1
    print("all pairs pass WCAG AA — palette verified")
    return 0


if __name__ == "__main__":
    sys.exit(main())
