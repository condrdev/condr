# Condr website

[condr.dev](https://condr.dev) 的官网与文档站，使用 Astro + Starlight。Cloudflare Pages 通过 GitHub 集成自动部署：root directory `website`，build command `pnpm build`，output `dist`。

## 本地开发

```sh
cd website
pnpm install --frozen-lockfile
pnpm dev
```

## 构建

```sh
pnpm build
pnpm preview
```

## 内容

- `src/pages/index.astro`：官网首页。
- `src/styles/home.css`：首页样式。
- `src/content/docs/docs/`：文档，对应 `/docs/`。
- `public/`：项目图标、真实产品截图，以及 `install.sh` / `install.ps1`（README 里 `curl -fsSL https://condr.dev/install.sh | sh` 用到的在线安装脚本）。

产品文案和资源以仓库根目录的 README 为准。
