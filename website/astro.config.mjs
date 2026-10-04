// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';

export default defineConfig({
  site: 'https://condr.dev',
  // The hero screenshot is imported from ../assets, outside the Vite root.
  vite: { server: { fs: { allow: ['..'] } } },
  // Only the docs have a Chinese version, so its locale root goes to its docs landing page.
  redirects: { '/zh-cn': '/zh-cn/docs/' },

  integrations: [
    starlight({
      title: { en: 'Condr Docs', 'zh-CN': 'Condr 文档' },
      favicon: '/condr.svg',
      // The site title links to /docs/ (or /zh-cn/docs/), not the marketing home page.
      routeMiddleware: './src/routeData.ts',
      description: 'Agents that never hang up. One window for every agent, local and remote. Disconnect anytime, pick up where you left off.',
      // English docs at /docs/, Chinese at /zh-cn/docs/.
      defaultLocale: 'root',
      locales: {
        root: { label: 'English', lang: 'en' },
        'zh-cn': { label: '简体中文', lang: 'zh-CN' },
      },
      sidebar: [
        {
          label: 'Start here',
          translations: { 'zh-CN': '从这里开始' },
          items: [
            { slug: 'docs/start/getting-started' },
            { slug: 'docs/start/install' },
            { slug: 'docs/start/concepts' },
            { slug: 'docs/start/why' },
          ],
        },
        {
          label: 'Using Condr',
          translations: { 'zh-CN': '使用 Condr' },
          items: [
            { slug: 'docs/using/overview' },
            { slug: 'docs/using/workspaces' },
            { slug: 'docs/using/agents' },
            { slug: 'docs/using/changes' },
            { slug: 'docs/using/remote' },
            { slug: 'docs/using/automation' },
            { slug: 'docs/using/preferences' },
          ],
        },
        {
          label: 'Reference',
          translations: { 'zh-CN': '参考' },
          items: [{ slug: 'docs/reference/cli' }, { slug: 'docs/reference/configuration' }, { slug: 'docs/reference/keybindings' }],
        },
        {
          label: 'Understanding and help',
          translations: { 'zh-CN': '理解与排障' },
          items: [{ slug: 'docs/help/troubleshooting' }, { slug: 'docs/help/security' }, { slug: 'docs/help/architecture' }],
        },
      ],
    }),
  ],
});
