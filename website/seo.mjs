export const defaultSiteUrl = 'https://dghilardi.github.io/lekton/';

export function normalizeSiteUrl(value = defaultSiteUrl) {
  const url = new URL(value);
  if (url.protocol !== 'https:' || url.username || url.password || url.search || url.hash) {
    throw new Error('Website URL must be an absolute HTTPS URL without credentials, query or fragment');
  }
  url.pathname = url.pathname.replace(/\/?$/, '/');
  return url.href;
}

export function createSeo(lang, translations, siteUrl) {
  const englishUrl = normalizeSiteUrl(siteUrl);
  const italianUrl = new URL('it/', englishUrl).href;
  const canonicalUrl = lang === 'en' ? englishUrl : italianUrl;
  const structuredData = {
    '@context': 'https://schema.org',
    '@graph': [
      { '@type': 'WebSite', '@id': `${englishUrl}#website`, name: 'Lekton', url: englishUrl, inLanguage: ['en', 'it'] },
      {
        '@type': 'WebPage', '@id': `${canonicalUrl}#webpage`, url: canonicalUrl,
        name: translations.title, description: translations.description, inLanguage: lang,
        isPartOf: { '@id': `${englishUrl}#website` }, about: { '@id': `${englishUrl}#software` },
      },
      {
        '@type': 'SoftwareApplication', '@id': `${englishUrl}#software`, name: 'Lekton',
        description: translations.description, applicationCategory: 'DeveloperApplication',
        url: englishUrl, sameAs: 'https://github.com/dghilardi/lekton',
        releaseNotes: 'https://github.com/dghilardi/lekton/releases',
        license: 'https://github.com/dghilardi/lekton/blob/main/LICENSE',
      },
    ],
  };
  return {
    canonicalUrl, englishUrl, italianUrl,
    shareImageUrl: new URL('assets/social.png', englishUrl).href,
    ogAlternateLocale: lang === 'en' ? 'it_IT' : 'en_US',
    // JSON-LD is script content, not HTML text. Prevent a translated '<' ending the script.
    structuredData: JSON.stringify(structuredData).replace(/</g, '\\u003c'),
  };
}
