<p align="center">
  <img src="https://github.com/user-attachments/assets/5eb9af15-c470-4ec9-925e-5e8b0db66950" alt="Muuuxy's logo" width="320" />
</p>

<p align="center">MUUUXY 🐮</p>

<p align="center">
  A simple M3U proxy for your browser-based video players.
</p>

## What is it?

MUUUXY rewrites `.m3u`/`.m3u8` HLS playlists and media segments on the fly, enabling seamless video playback in web players like HLS.js — with built-in CORS headers, DNS safety, and simple public proxy links.

## Features

- Public proxy endpoint for `.m3u`/`.m3u8` playlists
- Rewrites HLS segment URLs through your domain
- CORS and browser-player friendly

## Example

Each request to `/proxy` must follow this format:

```/proxy?key={id}?ur={encoded_url}```

Where:

- `{id}` is a user-bound ID (not yet implemented)
- `{encoded_url}` is a percent-encoded `.m3u8` URL (e.g., via `encodeURIComponent()`)

```bash
curl "http://localhost:3000/proxy?key=test&url=https%3A%2F%2Ftest-streams.mux.dev%2Fx36xhzz%2Fx36xhzz.m3u8"
```

## Development

```bash
cargo run
````

## 📄 License

MIT
