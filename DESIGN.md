# BestOfBurst — Grease Pencil Design System

> **Version** 1.0 · **Theme** Grease Pencil (light table + darkroom) · **Product** BestOfBurst desktop photo culler
> This file is the single source of truth for the Grease Pencil UI. Implement from it directly: tokens, components, marks, screens and rules are all here.

---

## 1. Concept

For about a hundred years, photographers culled by laying film on a **light table**, circling keepers in **red grease pencil**, and crossing out rejects. Grease Pencil turns that ritual into the interface.

- The app is a **light table** (day) or a **darkroom** (night). Photos are the brightest, most colourful things on screen. The UI chrome is paper, ink and graphite.
- **Every decision is a hand-drawn mark.** There are no badges, pills or status chips. A circle, a wave, a cross or a question mark reads at thumbnail size, before you read a single word.
- The layout is **editorial and brutalist**: square corners, 2 px rules, expanded uppercase headlines, monospace metadata like the edge print on film.
- **Calm for long sessions.** One accent colour (grease red), no gradients, no drop shadows (except the loupe), and built-in rest breaks.

### Design principles

| # | Principle | What it means in practice |
|---|-----------|---------------------------|
| 01 | **Marks, not badges** | Three hand-drawn shapes plus "?" carry every decision. Colour supports the shape but never replaces it. |
| 02 | **The shoot has a shape** | A timeline/histogram shows where bursts and close calls happen. Navigate by time, not endless scroll. |
| 03 | **Detail without zooming** | The loupe shows the subject at 100 % beside the full frame. |
| 04 | **Day or night table** | Bright studio or dim room: one toggle swaps the whole surface. Rest breaks are part of the flow. |
| 05 | **Only the user's eyes on close calls** | Clear wins are circled automatically; the UI only asks for attention where it's genuinely needed. |

---

## 2. Colour

### 2.1 Core palette

| Token | Name | Hex | Use |
|-------|------|-----|-----|
| `--gp-table` | Light table | `#E6E4DE` | Day ground, primary background |
| `--gp-ink` | Darkroom | `#121211` | Text on day, chrome bars, night ground |
| `--gp-film` | Film base | `#1B1A18` | Film strips, thumbnail wells |
| `--gp-graphite` | Graphite | `#5C5A55` | Quiet text, secondary labels |
| `--gp-red` | Grease red | `#D42A1C` | Marks on paper, primary action |
| `--gp-safelight` | Safelight | `#FF5A40` | Marks on film / dark surfaces, nudges |
| `--gp-edge` | Edge print | `#D9A441` | Frame numbers on film (edge-code style) |
| `--gp-cyan` | Cyanotype | `#1F4FD8` | Focus ring, selection outline, links |

### 2.2 Supporting neutrals

| Token | Hex | Use |
|-------|-----|-----|
| `--gp-ink-2` | `#3C3B37` | Body text on day when ink is too heavy |
| `--gp-ash` | `#A9A69D` | Muted text on dark |
| `--gp-rule-day` | `#C4C1B8` | Hairline rules on day |
| `--gp-rule-dark` | `#2E2D2A` | Hairline rules on dark chrome |
| `--gp-rule-night` | `#33322F` | Hairline rules on night table |
| `--gp-rule-strong` | `#3A3936` | Card borders on dark |
| `--gp-bar-day` | `#8E8B82` | Histogram bars, day |
| `--gp-night-table` | `#1A1918` | Night table ground |
| `--gp-nudge-bg` | `#2A1512` | Safelight break banner background |
| `--gp-safelight-soft` | `#FF8A73` | Text/labels inside the safelight banner |
| `--gp-paper-white` | `#FFFFFF` | Text on grease-red buttons only |

### 2.3 Day / night surface map

The **Day table / Night table** toggle swaps one set of semantic tokens. Components only use the semantic names.

| Semantic token | Day table | Night table |
|----------------|-----------|-------------|
| `--surface` | `#E6E4DE` | `#1A1918` |
| `--on-surface` | `#121211` | `#E6E4DE` |
| `--muted` | `#5C5A55` | `#A9A69D` |
| `--rule` | `#C4C1B8` | `#33322F` |
| `--bar` | `#8E8B82` | `#5C5A55` |
| `--mark` | `#D42A1C` | `#FF5A40` |
| `--chrome` (header bars) | `#121211` | `#121211` |
| `--on-chrome` | `#E6E4DE` | `#E6E4DE` |

**Rule:** marks drawn **over a photo or film** always use **Safelight `#FF5A40`** (it holds contrast on mid-tone images). Marks on **paper** use **Grease red `#D42A1C`**.

### 2.4 Status colours

| Status | Mark | Day | Night / on film |
|--------|------|-----|-----------------|
| Keeper | Circle | `#D42A1C` | `#FF5A40` |
| Maybe | Wave underline | `#D42A1C` | `#FF5A40` |
| Set aside | Cross + 45 % photo opacity | `#121211` | `#E6E4DE` |
| Close call | "?" glyph | `#FF5A40` | `#FF5A40` |
| Selected | 2.5 px outline | `#1F4FD8` | `#1F4FD8` |

Status is **always shape first**; colour is reinforcement (colour-blind safe by construction).

### 2.5 CSS variables

```css
:root {
  /* core */
  --gp-table: #E6E4DE;
  --gp-ink: #121211;
  --gp-film: #1B1A18;
  --gp-graphite: #5C5A55;
  --gp-red: #D42A1C;
  --gp-safelight: #FF5A40;
  --gp-edge: #D9A441;
  --gp-cyan: #1F4FD8;

  /* neutrals */
  --gp-ink-2: #3C3B37;
  --gp-ash: #A9A69D;
  --gp-rule-day: #C4C1B8;
  --gp-rule-dark: #2E2D2A;
  --gp-rule-night: #33322F;
  --gp-rule-strong: #3A3936;
  --gp-night-table: #1A1918;
  --gp-nudge-bg: #2A1512;
  --gp-safelight-soft: #FF8A73;

  /* semantic — day table (default) */
  --surface: var(--gp-table);
  --on-surface: var(--gp-ink);
  --muted: var(--gp-graphite);
  --rule: var(--gp-rule-day);
  --bar: #8E8B82;
  --mark: var(--gp-red);
  --chrome: var(--gp-ink);
  --on-chrome: var(--gp-table);
  --focus: var(--gp-cyan);
}

[data-table="night"] {
  --surface: var(--gp-night-table);
  --on-surface: var(--gp-table);
  --muted: var(--gp-ash);
  --rule: var(--gp-rule-night);
  --bar: var(--gp-graphite);
  --mark: var(--gp-safelight);
}
```

---

## 3. Typography

### 3.1 Families

| Role | Family | Settings |
|------|--------|----------|
| Display / headlines / actions | **Archivo**, expanded | `font-stretch: 125%`, weight 700–900, UPPERCASE |
| Body / reading | **Archivo**, normal width | `font-stretch: 100%`, weight 400–500 |
| Metadata / numbers / keys | **Martian Mono**, condensed | `font-stretch: 87.5%`, weight 400–600 |

```html
<link href="https://fonts.googleapis.com/css2?family=Archivo:wdth,wght@62..125,100..900&family=Martian+Mono:wdth,wght@75..112.5,300..700&display=swap" rel="stylesheet">
```

```css
body  { font-family: 'Archivo', system-ui, sans-serif; }
.xp   { font-stretch: 125%; text-transform: uppercase; }      /* expanded display */
.mono { font-family: 'Martian Mono', monospace; font-stretch: 87.5%; }
```

For the desktop app, **bundle both fonts locally** (the app is offline-first). Both are SIL OFL.

### 3.2 Type scale

| Token | Size / line-height | Weight | Style | Used for |
|-------|--------------------|--------|-------|----------|
| `display-xxl` | 220 / 0.80 | 900 | xp, tabular-nums, −0.04em | Done screen count ("1,000") |
| `display-xl` | 148 / 0.82 | 800 | xp, tabular-nums, −0.03em | Processing frame counter |
| `display-l` | 96 / 0.85 | 800 | xp, −0.03em | Close-call count on Review |
| `display-m` | 84 / 0.92 | 800 | xp, upper, −0.02em | Home hero headline |
| `display-s` | 76 / 0.90 | 800 | xp, tabular-nums | Keep target number |
| `headline` | 26–30 / 1.15 | 800 | xp, upper | Section titles, Done subline |
| `title` | 20–24 / 1.2 | 700–800 | xp, upper | Card titles, shoot names |
| `label` | 13–16 / 1.2 | 800 | xp, upper, +0.03em | Buttons, nav, brand wordmark |
| `body-l` | 16 / 1.45 | 400 | — | Explanations |
| `body` | 14 / 1.45 | 400 | — | Descriptions, helper text |
| `meta` | 10.5–11 / 1.3 | 500–600 | mono, upper, +0.08em | Eyebrows, EXIF, frame numbers, status lines |

**Rules**
- Headlines and actions are **UPPERCASE expanded**. Body text is never uppercase.
- Every number that changes (counts, scores, progress) uses `font-variant-numeric: tabular-nums`.
- The minimum text size is **10 px mono** (metadata only) and **13 px** for anything else.
- Eyebrow pattern: `meta` mono label above a display number (e.g. `NEEDS YOUR EYES` → `184`).

---

## 4. Layout, spacing and shape

### 4.1 Grid and spacing

- Base unit: **4 px**. Common steps: `4, 6, 8, 10, 12, 14, 16, 18, 20, 22, 24, 28, 32, 36, 40, 48, 56, 64`.
- Page padding: **48 px** on wide screens, **24 px** on panels, **20 px** on dense views (Focus).
- Section gaps: **32–40 px** between major blocks; **14–22 px** inside a panel.
- Layout is **flex-wrap, panel based**: a main work area (`flex: 999 1 560–600px`) plus a side panel (`flex: 1 1 300–420px`) that drops below on narrow windows.
- Contact sheet: **6-column** frame grid per time row; Done stats use a **3-column** grid.

### 4.2 Shape

| Property | Value |
|----------|-------|
| Corner radius | **0** everywhere (buttons, cards, inputs, thumbnails) |
| Exception | Key caps `3px`; status dots `50%` |
| Rules | **1 px** hairlines between regions; **1.5–2 px** for card borders and strong dividers |
| Shadows | **None**, except the loupe (a floating lens) |
| Gradients | **None** in chrome. Photos are the only gradients on screen. |

### 4.3 App frame

```
┌───────────────────────────────────────────────────────────────┐
│ CHROME BAR  (#121211, 1px #2E2D2A dividers between cells)     │
│ [logo+wordmark] │ SHOOT NAME │ ……… │ GPU · status │ [DELIVER] │
├───────────────────────────────────────────┬───────────────────┤
│ MAIN (surface)                            │ SIDE PANEL        │
│ timeline / sheet / preview                │ (1px rule left)   │
│                                           │ count · action    │
└───────────────────────────────────────────┴───────────────────┘
```

- The header is a row of **cells** separated by vertical 1 px rules, not floating buttons.
- The primary action in the header is a full-height **grease-red cell** (e.g. `DELIVER`).

---

## 5. The marks (signature element)

All marks are inline SVG strokes with `fill="none"` and `stroke-linecap="round"`, slightly irregular so they read as hand-drawn. **Never use perfect geometric circles.**

| Mark | Meaning | Spec |
|------|---------|------|
| **Circle** | Keeper | Open loop that overshoots its start. Stroke 2.2–6 px depending on size. |
| **Wave** | Maybe | One soft wave stroke under the frame. |
| **Cross** | Set aside | Two slightly bowed diagonal strokes; the photo drops to **45 % opacity**. |
| **?** | Close call | Archivo xp 900, 26 px, Safelight, top-right of the frame. |

### Reference paths

```svg
<!-- Keeper circle — hero size (viewBox 0 0 300 110, preserveAspectRatio="none") -->
<path d="M40 70 C 20 20, 250 0, 285 45 C 310 85, 120 112, 30 88 C 0 78, 18 40, 70 30"
      fill="none" stroke="#D42A1C" stroke-width="5" stroke-linecap="round"/>

<!-- Keeper circle — thumbnail (viewBox 0 0 120 100) -->
<path d="M22 60 C 10 18, 90 4, 100 40 C 108 74, 44 98, 14 74 C 2 62, 12 36, 36 26"
      fill="none" stroke="#FF5A40" stroke-width="3.5" stroke-linecap="round"/>

<!-- Logo / icon circle (24 px) -->
<path d="M5 9c-1-6 13-7 14-2s-11 7-13 3"
      fill="none" stroke="#D42A1C" stroke-width="2.4" stroke-linecap="round"/>
```

**Usage rules**
- Use the hero circle around a **word or number** to emphasise it ("Circle the best.", "1,000"). Use it once per screen at most.
- Thumbnail marks are scaled with `preserveAspectRatio="none"` so they hug any frame ratio.
- **Animating a mark:** draw it on with `stroke-dasharray` / `stroke-dashoffset` over **350–500 ms ease-out**, only at the moment of the decision. Never loop a mark.

---

## 6. Components

### 6.1 Buttons

| Variant | Spec |
|---------|------|
| **Primary** | bg `#D42A1C`, text `#FFFFFF`, label style (xp 800 upper), height **52–68 px**, padding 0 22–24 px, radius 0, no border. Shortcut key right-aligned (`justify-content: space-between`). |
| **Secondary (on dark)** | transparent, `2px solid #E6E4DE`, text `#E6E4DE`. |
| **Secondary (on day)** | bg `#121211`, text `#E6E4DE`, or `2px solid #121211` outline. |
| **Inverse** | bg `#E6E4DE`, text `#121211`, on dark chrome. |
| **Alert outline** | transparent, `2px solid #FF5A40`, text `#FF8A73` (e.g. "Move circle to B"). |
| **Mono small** | height 40 px, `1.5px solid currentColor`, Martian Mono 600 11 px (banner actions like `DONE`). |

States: hover darkens the fill by about 8 % (red → `#B82316`) or inverts the outline; pressed `translateY(1px)`; disabled 40 % opacity. Focus: `outline: 2.5px solid #1F4FD8; outline-offset: 2px`.

### 6.2 Key cap

```css
.key {
  display: inline-flex; align-items: center; justify-content: center;
  min-width: 26px; height: 26px; padding: 0 6px;
  border: 1.5px solid currentColor; border-radius: 3px;
  font: 600 11px 'Martian Mono', monospace; box-sizing: border-box;
}
```

Every primary action shows its keyboard shortcut: `↵` Keep & next, `N` next close call, `P` move circle, `C` compare, `X` set aside, `CTRL E` deliver, `Esc` back.

### 6.3 Segmented choice (shot type)

- A grid of square cells with 1 px `#3A3936` rules between them.
- Selected: bg `#E6E4DE`, text `#121211`, `aria-pressed="true"`.
- Unselected: transparent, text `#E6E4DE`, sub-label in mono `#A9A69D`.
- Each cell: xp 800 15 px title plus a mono 10 px descriptor (e.g. `GENERAL` / `BALANCED`).

### 6.4 Film strip / thumbnail

- Well bg `#1B1A18`, sprocket holes as short 2 px dashes top and bottom.
- Frame number in **Edge print `#D9A441`**, mono 10 px (e.g. `▸ 4824 12A`).
- Stack count `×5` in mono at bottom-left.
- Selected: `outline: 2.5px solid #1F4FD8`.
- Set aside: 45 % opacity plus a cross.

### 6.5 Ruler bar (metric comparison)

- Track: 2 px rule in `--rule` with tick marks every 10 %.
- Keeper value: solid bar in `--mark`.
- Runner-up: a 3 px vertical tick in `--on-surface`.
- Label row: title on the left in body 14 px, delta on the right in mono 11 px (`keeper +4`, `even`).

### 6.6 Timeline / histogram

- 80 vertical bars in `--bar`. Bars containing close calls are in `--mark`.
- Time labels in mono `--muted`. Clicking a bar jumps the sheet to that time row.

### 6.7 Day/night table toggle

- A mono label `DAY TABLE` / `NIGHT TABLE` plus a square track with a knob that moves left/right.
- `aria-pressed` reflects night mode. It swaps `data-table` on the root.

### 6.8 Safelight break banner

```
bg #2A1512 · border-left 3px solid #FF5A40 · role="status"
[SAFELIGHT BREAK (mono, #FF8A73)]  22 min in — look at something far away for 20 seconds.  [PLAY WITH GLIM] [DONE]
```

- Appears after **20+ minutes** of continuous review. It never blocks input and is always dismissable.

### 6.9 Loupe

- The **only** element with a shadow: a floating square lens showing the subject at 100 %.
- `2px solid #121211`, `box-shadow: 0 18px 40px rgba(0,0,0,0.35)`.

### 6.10 Checklist row (delivery options)

- Row with a top rule, a square 20 px checkbox (accent `#D42A1C`), an xp 15 px title, a body 14 px description in `#3C3B37`, and an optional mono path line.

---

## 7. Screens

| # | Screen | Key composition |
|---|--------|-----------------|
| 1 | **Home (Main)** | Hero "TEN THOUSAND FRAMES. CIRCLE THE BEST." with the hero circle around "Circle". Drop-folder zone, recent shoots with `Resume →` in red. Dark side panel: Glim greeting card, Keep target (76 px), shot-type segmented grid, engine (GPU/CPU), full-width red `DEVELOP ↵`. |
| 2 | **Processing** | Dark full screen. Giant frame counter (148 px), step list (scan → measure → group → rank) with the current step in Safelight, a live film strip, sleepy Glim in the header, `PAUSE & SAVE` and `OPEN CONTACT SHEET →`. |
| 3 | **Review (contact sheet)** | Timeline histogram, then time rows of 6 marked frames. Side panel: Glim card, `184` close calls, red `OPEN THE LOUPE N`, and the selected-frame detail. Day/night toggle in the header. |
| 4 | **Focus (loupe)** | Night ground. Group counter `37 / 184`, safelight break banner, large preview plus film strip, decision panel with Glim, ruler bars, `MOVE CIRCLE TO B P` and red `KEEP & NEXT ↵`. |
| 5 | **Compare** | Wipe and blink between the keeper and the runner-up; `1` / `2` keys choose. |
| 6 | **Done** | Light table. `1,000` at 220 px circled in red, "keepers, circled from 10,214 frames", a 3-column stat grid, cheering Glim with "spend treats", and the delivery checklist with red `DELIVER 1,000 CTRL E`. |
| 7 | **Glim's corner** | Light table habitat, pencil Glim (290 px), ruler-style Joy/Energy bars, Pet / Treat / Nap, and the "Sharp Eye" film-strip mini-game (circle the one sharp frame). |

---

## 8. Glim — the pet (pencil look)

Glim is BestOfBurst's own original mascot: a small lens-hood body with a **camera-aperture eye**. In Grease Pencil it's drawn as an **ink line drawing on paper**, matching the marks.

| Prop | Values |
|------|--------|
| `look` | `pencil` (this theme) |
| `surface` | `day` → paper `#F3F2EE`, ink `#121211`, red `#D42A1C` · `night` → paper `#2A2926`, ink `#E6E4DE`, red `#FF5A40` |
| `mood` | `idle`, `happy`, `sleepy`, `cheer` |
| `size` | 40–600 px |

**Mood map**

| Where | Mood | Surface |
|-------|------|---------|
| Home side panel | happy | night |
| Processing header | sleepy | night |
| Review side panel | idle | follows the day/night toggle |
| Focus decision panel | happy | night |
| Done | cheer (a red grease circle draws around it) | day |
| Glim's corner | reacts to play | day |

**Behaviour rules**
- Glim **reacts; it never blocks**. No modals, no sound, no stealing focus.
- Each settled close call earns **one treat**. Treats are spent in Glim's corner.
- Rest-break nudges come from Glim and are always dismissable.
- Motion is calm: a bob of 2–4 s loops, a blink every few seconds, z's when sleepy.
- Every Glim placement is a link to Glim's corner with an `aria-label`.

---

## 9. Motion

| Use | Duration | Easing |
|-----|----------|--------|
| Mark draw-on (decision) | 350–500 ms | ease-out |
| Panel / selection change | 160–200 ms | ease |
| Progress fill | 400 ms | ease |
| Glim idle loops | 2–4 s | sine (SMIL) |
| Compare blink | 450 ms interval | step |

- No parallax, no bouncing UI, no looping chrome animation.
- Respect `prefers-reduced-motion: reduce`: disable draw-on (show the final mark), stop Glim loops, and keep blink manual.

---

## 10. Accessibility

- **Contrast:** ink on table is about 15:1; grease red on table is about 4.0:1, so use red text only at ≥ 19 px bold (AA large) and for marks; white on grease red is about 5.1:1. Use Safelight on dark only.
- **Never colour alone:** every status has a distinct shape plus an `aria-label` (`"Frame 4824, circled, close call"`).
- **Keyboard first:** every action has a shortcut and a visible key cap. Focus ring is Cyanotype 2.5 px.
- **Targets:** at least 40 px tall for small controls; primary actions are 52–68 px.
- `role="status"` for the break banner and Glim's speech; `aria-pressed` for toggles and segmented choices.
- Night table is for dim rooms, not decoration. Both tables must pass the same contrast checks.

---

## 11. Do / Don't

| Do | Don't |
|----|-------|
| Let photos be the only colourful thing | Add gradients, glows or tinted panels |
| Use the hand-drawn marks for status | Use pills, badges or coloured dots for decisions |
| Square corners, 1–2 px rules | Round corners or card drop-shadows |
| One hero circle per screen | Circle everything |
| UPPERCASE expanded for headlines and actions | Uppercase body text |
| Show shortcut keys on actions | Hide actions behind hover-only menus |
| Safelight on film/dark, grease red on paper | Swap them; red on mid-tone photos disappears |
| Keep Glim small and reactive | Let Glim cover photos or interrupt a decision |

---

## 12. Token export (JSON)

```json
{
  "color": {
    "table": "#E6E4DE", "ink": "#121211", "film": "#1B1A18", "graphite": "#5C5A55",
    "red": "#D42A1C", "safelight": "#FF5A40", "edge": "#D9A441", "cyan": "#1F4FD8",
    "ink2": "#3C3B37", "ash": "#A9A69D", "ruleDay": "#C4C1B8", "ruleDark": "#2E2D2A",
    "ruleNight": "#33322F", "ruleStrong": "#3A3936", "nightTable": "#1A1918",
    "nudgeBg": "#2A1512", "safelightSoft": "#FF8A73"
  },
  "surface": {
    "day":   { "surface": "#E6E4DE", "onSurface": "#121211", "muted": "#5C5A55", "rule": "#C4C1B8", "bar": "#8E8B82", "mark": "#D42A1C" },
    "night": { "surface": "#1A1918", "onSurface": "#E6E4DE", "muted": "#A9A69D", "rule": "#33322F", "bar": "#5C5A55", "mark": "#FF5A40" }
  },
  "font": {
    "display": { "family": "Archivo", "stretch": "125%", "transform": "uppercase" },
    "body":    { "family": "Archivo", "stretch": "100%" },
    "mono":    { "family": "Martian Mono", "stretch": "87.5%" }
  },
  "radius": { "default": 0, "key": 3 },
  "rule": { "hair": 1, "strong": 2 },
  "space": [4, 6, 8, 10, 12, 14, 16, 18, 20, 22, 24, 28, 32, 36, 40, 48, 56, 64],
  "control": { "small": 40, "primary": 52, "hero": 64, "deliver": 68 },
  "motion": { "mark": 450, "ui": 180, "progress": 400, "blink": 450 }
}
```

### Implementation notes (Tauri / egui)

- **Tauri (MVP):** copy section 2.5 into `theme.css`. Toggle night with `document.documentElement.dataset.table = 'night'` and persist the choice in the app's JSON state.
- **egui (prototype):** map the semantic tokens to `egui::Visuals`: `panel_fill = surface`, `override_text_color = on_surface`, `selection.stroke = cyan 2.5`, `widgets.*.rounding = 0`, `window_shadow = none`. Draw the marks with `Painter::add(Shape::CubicBezier)` using the reference paths in section 5.
