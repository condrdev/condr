// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';

export default defineConfig({
  site: 'https://condr.dev',
  // The hero screenshot is imported from ../assets, outside the Vite root.
  vite: { server: { fs: { allow: ['..'] } } },
  // Only the docs have a Chinese version; its site title links to the locale root.
  redirects: { '/zh-cn': '/' },

  integrations: [
    starlight({
      title: { en: 'Condr Docs', 'zh-CN': 'Condr 文档' },
      favicon: '/condr.svg',
      description: 'Keep your agents running. One window for all your agents, local or remote.',
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
            { slug: 'docs/getting-started' },
            { slug: 'docs/install' },
            { slug: 'docs/concepts' },
            { slug: 'docs/why' },
          ],
        },
        {
          label: 'Using Condr',
          translations: { 'zh-CN': '使用 Condr' },
          items: [
            { slug: 'docs/workspaces' },
            { slug: 'docs/agents' },
            { slug: 'docs/changes' },
            { slug: 'docs/remote' },
            { slug: 'docs/automation' },
          ],
        },
        {
          label: 'Reference',
          translations: { 'zh-CN': '参考' },
          items: [{ slug: 'docs/cli' }, { slug: 'docs/configuration' }, { slug: 'docs/keybindings' }],
        },
        {
          label: 'Understanding and help',
          translations: { 'zh-CN': '理解与排障' },
          items: [{ slug: 'docs/troubleshooting' }, { slug: 'docs/security' }, { slug: 'docs/architecture' }],
        },
      ],
    }),
  ],
});
