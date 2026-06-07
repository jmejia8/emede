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

- Read-only rendering — no editor, distraction-free layout
- In-process Comrak markdown-to-HTML conversion (no external tools)
- Live reload when an external editor saves the open file
- Bundled MathJax for offline math rendering
- Configurable font, text color, and background
- Presets: Light, Sepia, Dark, Gruvbox
- Settings stored in `~/.config/emede/settings.json`

## Math syntax

- Inline: `$...$` or `\(...\)`
- Display: `$$...$$` or `\[...\]`
- Fenced: ` ```math ` code blocks

## Settings

Press `Ctrl+,` or click the gear icon (top-right on hover) to open settings.

Default typography uses Literata with Source Serif 4 and Noto Serif fallbacks.

## License

MIT
