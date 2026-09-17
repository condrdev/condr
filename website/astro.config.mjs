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
      description: 'A calm command center for agent work.',
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