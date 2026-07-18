# LumaGet

LumaGet is a clean-room, open-source download manager for Windows with a modern desktop UI and a Chromium browser extension.

## Development

Requirements: Node.js 20+, pnpm 11+, Rust 1.80+, and Visual Studio 2022 Build Tools with the **Desktop development with C++** workload.

```bash
pnpm install
pnpm tauri dev
```

Build the browser extension with `pnpm extension:build`. Load `extension/dist` as an unpacked extension in Chrome or Edge.

## Scope

- HTTP/HTTPS downloads with pause, resume, retry and cancellation
- Parallel HTTP Range segments with persistent part-file resume and safe merging
- Persistent queue, history, categories, search and bandwidth controls
- Chromium Manifest V3 integration with context-menu sending and download interception
- Original glass-style light/dark interface

Basic media discovery recognizes direct video/audio resources and HLS/DASH manifest URLs exposed by a page. DRM-protected media and platform access-control bypasses are intentionally unsupported.

LumaGet is not affiliated with Neat Download Manager. It does not contain or reuse proprietary NDM source code or assets.

## License

MIT
