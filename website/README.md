# Condr website

[condr.dev](https://condr.dev) 的官网与文档站，使用 Astro + Starlight。部署由 `.github/workflows/website.yml` 完成，在 GitHub Actions 里构建并用 `wrangler pages deploy dist` 上传到 Cloudflare Pages 项目 `condr`（Direct Upload，不连接 GitHub，不占 Pages 构建配额），分两个站：

- **正式站 condr.dev**：最新正式版的文档、安装脚本和下载页，从它的 tag 构建。`release.yml` 和 `companion.yml` 发布完 release 后调用 `website.yml` 部署，所以文档不会写到用户还装不到的功能。
- **预览站 next.condr.pages.dev**：`main`，也就是 nightly 的文档。push 到 `main` 且改动 `website/**`、`assets/**` 或 `script/install-condr.*` 时部署；每页顶部有横幅说明，并且 `noindex`。构建时设 `CONDR_SITE_CHANNEL=next`，见 `src/channel.ts`。

两个站的下载页和 `/version.txt` 都写最新已发布的桌面版和 Companion 版本，由 workflow 通过 `CONDR_RELEASE_VERSION` 和 `CONDR_COMPANION_VERSION` 传入，不看构建所用 ref 里的版本文件；本地构建没有这两个变量时才读 `Cargo.toml` 和 `mobile/version.xcconfig`。已发布版本的文档要紧急修正时，从 release tag 拉一个分支、cherry-pick 修正，再运行 `gh workflow run website.yml -f target=production -f ref=<分支>`。

需要仓库 secrets `CLOUDFLARE_API_TOKEN`（权限：Cloudflare Pages: Edit）和 `CLOUDFLARE_ACCOUNT_ID`。

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

- `src/layouts/Site.astro`：首页和下载页共用的 head、页眉、页脚。
- `src/pages/index.astro`：官网首页。
- `src/pages/download.astro`：下载页 `/download/`。链接指向最新已发布的 release 的文件（见上），所以 `main` 上先改版本号不会让页面指向还不存在的文件。
- `src/styles/home.css`：首页和下载页样式。
- `src/content/docs/docs/`：英文文档，对应 `/docs/`，按侧栏分组放在 `start/`、`using/`、`reference/`、`help/` 四个子目录里，URL 带上子目录，例如 `/docs/using/workspaces/`。`src/content/docs/zh-cn/docs/` 是同样结构的中文版，对应 `/zh-cn/docs/`，改英文文档时同步改中文；缺中文版的页面由 Starlight 显示英文原文并提示未翻译。官网首页和下载页只有英文，`/zh-cn/` 重定向到 `/zh-cn/docs/`，文档页左上角的标题也链到各自语言的 `/docs/`，由 `src/routeData.ts` 设置。
- `public/`：只放网站自己的静态文件（目前只有 `_headers`）。`install.sh` / `install.ps1`、`condr.svg`、`agents/*.svg` 由 `prebuild` 从仓库根的 `script/` 和 `assets/` 拷入，已 gitignore；要改就改原件。首页截图从 `../assets/screenshots/` import，由 Astro 优化输出。其中 `phone-*.png` 是手动截的；`hero`、`hero-with-phone`（仓库 README 用，把 `phone-agents.png` 放在主图前）、`needs-you`、`panes`、`diff`、`preview`、`history`、`ports`、`transfers` 由 GUI 离屏渲染生成，在 macOS 上于仓库根目录运行 `CONDR_GUI_WEBSITE_SCREENSHOTS=assets/screenshots cargo run -p condr-gui --features test-support` 重新生成（需要装好 Claude Code、Codex 和 lazygit，前两者要信任本仓库；场景见 `crates/condr-gui/src/app/screenshots/website.rs`）。

产品文案和资源以仓库根目录的 README 为准。
