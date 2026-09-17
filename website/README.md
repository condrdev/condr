# Condr website

[condr.dev](https://condr.dev) 的官网与文档站，使用 Astro + Starlight。部署由 `.github/workflows/website.yml` 完成：push 到 `main` 且改动 `website/**` 或 `script/install-condr.*` 时，在 GitHub Actions 里构建并用 `wrangler pages deploy dist` 上传到 Cloudflare Pages 项目 `condr`（Direct Upload，不连接 GitHub，不占 Pages 构建配额）。需要仓库 secrets `CLOUDFLARE_API_TOKEN`（权限：Cloudflare Pages: Edit）和 `CLOUDFLARE_ACCOUNT_ID`。

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
- `public/`：项目图标、真实产品截图。`install.sh` / `install.ps1` 由 `prebuild` 从 `../script/install-condr.*` 拷入，已 gitignore；要改就改 `script/` 里的源文件。

产品文案和资源以仓库根目录的 README 为准。
