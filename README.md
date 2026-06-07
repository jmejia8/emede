# emede

Immersive, read-only markdown viewer powered by [Tauri](https://tauri.app), [pandoc](https://pandoc.org), and bundled [MathJax](https://www.mathjax.org).

## Requirements

- [pandoc](https://pandoc.org) on `PATH` (e.g. `pacman -S pandoc`)
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
- Pandoc converts markdown to HTML (with LaTeX math)
- Bundled MathJax for offline math rendering
- Configurable font, text color, and background
- Presets: Light, Sepia, Dark
- Settings stored in `~/.config/emede/settings.json`

## Settings

Press `Ctrl+,` or click the gear icon (top-right on hover) to open settings.

Default typography uses Literata with Source Serif 4 and Noto Serif fallbacks.

## License

MIT
