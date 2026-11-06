import { defineRouteMiddleware } from '@astrojs/starlight/route-data';
import { nightly, nightlyBanner } from './channel';

// The site title links to the docs landing page of the current language instead of
// the site root, which is the marketing home page.
export const onRequest = defineRouteMiddleware((context) => {
  const route = context.locals.starlightRoute;
  route.siteTitleHref = route.locale ? `/${route.locale}/docs/` : '/docs/';
  // The nightly's docs say so on every page and stay out of search results.
  if (nightly) {
    route.entry.data.banner ??= {
      content: route.lang === 'zh-CN' ? nightlyBanner['zh-CN'] : nightlyBanner.en,
    };
    route.head.push({ tag: 'meta', attrs: { name: 'robots', content: 'noindex' } });
  }
});
