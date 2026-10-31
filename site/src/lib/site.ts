export const AUTHOR = {
  '@type': 'Person',
  name: 'Saiful Islam',
  url: 'https://github.com/adar2378',
  sameAs: ['https://github.com/adar2378'],
} as const;

export const PUBLISHER = {
  '@type': 'Organization',
  name: 'claude-garage',
  url: 'https://claude-garage.xyz',
  logo: { '@type': 'ImageObject', url: 'https://claude-garage.xyz/favicon.svg' },
} as const;

export const isoDate = (d: Date) => d.toISOString().slice(0, 10);
export const longDate = (d: Date) =>
  d.toLocaleDateString('en-US', { year: 'numeric', month: 'long', day: 'numeric', timeZone: 'UTC' });
