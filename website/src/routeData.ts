import { defineRouteMiddleware } from '@astrojs/starlight/route-data';

// The site title links to the docs landing page of the current language instead of
// the site root, which is the marketing home page.
export const onRequest = defineRouteMiddleware((context) => {
  const route = context.locals.starlightRoute;
  route.siteTitleHref = route.locale ? `/${route.locale}/docs/` : '/docs/';
});
