// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';

export default defineConfig({
  site: 'https://condr.dev',
  // The hero screenshot is imported from ../assets, outside the Vite root.
  vite: { server: { fs: { allow: ['..'] } } },

  integrations: [
    starlight({
      title: 'Condr Docs',
      favicon: '/condr.svg',
      description: 'Keep your agents running. One window for all your agents, local or remote.',
      sidebar: [
        {
          label: 'Start here',
          items: [{ slug: 'docs/getting-started' }, { slug: 'docs/concepts' }],
        },
        {
          label: 'Reference',
          items: [{ slug: 'docs/architecture' }],
        },
      ],
    }),
  ],
});