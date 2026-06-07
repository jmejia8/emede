# emede

Immersive, read-only markdown viewer powered by [Tauri](https://tauri.app), [Comrak](https://github.com/kivikakk/comrak), and bundled [MathJax](https://www.mathjax.org).

## Requirements

- Rust toolchain and Node.js (for building)

## Usage

```bash
# development
npm install
npm run tauri dev -- test.md

# release binary
npm run tauri build
./src-tauri/target/release/emede document.md
```

Open a markdown file by passing it as the first argument:

```bash
emede notes/lecture.md
```

## Features

- Read-only rendering — no editor, distraction-free reader layout
- In-process Comrak markdown-to-HTML conversion (no external tools)
- GFM-style extensions: tables, task lists, strikethrough, autolinks
- YAML/front matter shown as a fenced block; document title from `title:` or first `#` heading
- Bundled MathJax for offline math rendering
- Table of contents for documents with headings (expand/collapse sections)
- External links open in the system browser
- Custom titlebar or native system window decorations
- Typography: separate body, title, and code fonts; font size and side margins
- Color presets: Light, Sepia, Dark, Gruvbox; font presets for common type stacks
- Settings persisted in `~/.config/emede/settings.json`

## Math syntax

- Inline: `$...$` or `\(...\)`
- Display: `$$...$$` or `\[...\]`
- Fenced: ` ```math ` code blocks

## Settings

Press `Ctrl+,` or click the gear icon (top-right on hover) to open settings.

Default typography uses Literata with Source Serif 4 and Noto Serif fallbacks.

## License

MIT
