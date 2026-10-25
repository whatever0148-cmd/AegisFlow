# Vendored browser libraries

The console loads nothing from a CDN at runtime; everything it needs is in this directory and is
served by aegis-daemon under `/vendor/`.

## AG Grid Community

| | |
|---|---|
| Package | `ag-grid-community` |
| Version | `36.2.0` (published to npm 2026-09-16T08:00:00Z) |
| License | MIT, see `ag-grid-community/LICENSE.txt` (the UMD bundle also inlines `ag-stack@36.2.0`, MIT, same repository) |
| Source | `npm pack ag-grid-community@36.2.0` → `https://registry.npmjs.org/ag-grid-community/-/ag-grid-community-36.2.0.tgz` |
| Tarball SHA-256 | `691720b60486ab2e8ebafb6f760b198a718c766c55d3c4e204264d44e636a816` |
| Tarball integrity (npm, verified) | `sha512-tK0Toj7t0fV/OMLPnDc0xlYKAqDbnCT3iJodYSn2TJ541caU8jPdYwWhMdW+AP2+pUNwFiJVgc04eJNfvBPsWw==` |

Files copied verbatim from the tarball's `package/` directory:

| File here | From | SHA-256 |
|---|---|---|
| `ag-grid-community/ag-grid-community.min.noStyle.js` | `package/dist/ag-grid-community.min.noStyle.js` | `3b05777f78aa10b6d7fd020bc2076936b889e5a0ff9c1a764fdbe6aa19456993` |
| `ag-grid-community/LICENSE.txt` | `package/LICENSE.txt` | `31636f0472dda16bba11a7b78204e1f677b77452092e185a65cddd45b6c9fb29` |

Why this bundle: it is the self-contained UMD build. It defines `window.agGrid` and registers
`AllCommunityModule` itself. The ESM build (`dist/package/main.esm.mjs`) imports the bare specifier
`ag-stack`, which a browser cannot resolve without a bundler or import map. The `noStyle` variant
is the right one for the Theming API. The default `ag-grid-community.min.js` also injects the legacy
`ag-theme-*` CSS and four `data:font/woff2` icon fonts when it loads, and a `font-src 'self'` CSP
blocks those fonts. The `noStyle` build leaves them out, and the Theming API still injects the
grid's own CSS at runtime. The console uses the
Theming API (`themeQuartz.withPart(colorSchemeDark).withParams({...})`, passed as the `theme` grid
option, checked against `dist/types/src/theming/**` in the package). It sets
`loadThemeGoogleFonts: false` and a system font stack, so the grid never fetches fonts.

To verify the vendored copy:

```sh
shasum -a 256 apps/aegis-console/vendor/ag-grid-community/*
```

To upgrade: run `npm pack ag-grid-community@<version>` in a scratch directory outside the repo,
check the tarball against `npm view ag-grid-community@<version> dist.integrity`, copy the same two
files, and update this table.
