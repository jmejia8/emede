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

- Read-only rendering — no editor, distraction-free reader layout.
- In-process Comrak markdown-to-HTML conversion (no external dependencies).
- GFM-style extensions: tables, task lists, strikethrough, autolinks.
- Bundled MathJax for offline math rendering.
- Table of contents for documents. 
- Typography and side margins customization.
- Color presets: Light, Sepia, Dark, Gruvbox; font presets for common type stacks.
- Settings persisted in `~/.config/emede/settings.json`
- Keybinding schemes: Vim, Emacs, and Common reader navigation.

## Math syntax

- Inline: `$...$` or `\(...\)`.
- Display: `$$...$$` or `\[...\]`.
- Fenced: ` ```math ` code blocks.

## Settings

Press `Ctrl+,` or click the gear icon (top-right on hover) to open settings.

Choose a keybinding scheme under **Keybindings**. Vim mode adds `j`/`k` line scrolling, `d`/`u` half-page movement, `f`/`b`/`Space` paging, and `gg`/`G` for top and bottom. Emacs mode uses `Ctrl+n`/`Ctrl+p` and `Ctrl+v`/`Alt+v`. Common reader mode uses `j`/`k`, `Space`/`Shift+Space`, and `Ctrl+Home`/`Ctrl+End`. All schemes also support `Ctrl+,` for settings, `Ctrl+Shift+T` for the table of contents, and `Escape` to close panels.

Default typography uses Literata with Source Serif 4 and Noto Serif fallbacks.

## License

MIT
