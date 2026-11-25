import rustSecretsMd from '../content/blog/rust-secrets-fast-builds-tiny-target-4s-tests.md?raw';
import goSizedMd from '../content/blog/rust-go-sized-web-binaries.md?raw';
import djangors10Md from '../content/blog/introducing-djangors-1-0.md?raw';

export interface BlogPost {
  slug: string;
  title: string;
  description: string;
  date: string;
  readingTime: string;
  tags: string[];
  cover: string;
  coverAlt: string;
  featured?: boolean;
  markdown: string;
}

export const posts: BlogPost[] = [
  {
    slug: 'rust-secrets-fast-builds-tiny-target-4s-tests',
    title: "The Rust Secrets Nobody Tells You: 4s Test Suites, 4 MiB Binaries, and a Hard-Capped target/",
    description:
      'Every real Cargo, nextest, rust-analyzer, and profile trick we used to cut a 747-test SQLite suite to 4.26s, eliminate 350 MiB of dead test binaries, and hard-cap target/ under 4 GiB.',
    date: '2026-11-25',
    readingTime: '8 min read',
    tags: ['Rust', 'Cargo', 'Testing', 'Performance'],
    cover: '/blog/rust-secrets-speed.png',
    coverAlt: 'Swiss technical editorial illustration of oxidized copper gears and parallel iron lanes on deep charcoal',
    featured: true,
    markdown: rustSecretsMd,
  },
  {
    slug: 'rust-go-sized-web-binaries',
    title: 'Shipping Go-Sized Web Binaries in Rust: How a Batteries-Included Framework Hits 4 MiB',
    description:
      'Why default Rust binaries balloon to 60+ MiB and how release profile tuning plus strict feature isolation produces 4.00 MiB minimal and 11.51 MiB full-stack static binaries.',
    date: '2026-11-24',
    readingTime: '6 min read',
    tags: ['Rust', 'Binary Size', 'Deployment'],
    cover: '/blog/go-sized-binaries.png',
    coverAlt: 'Industrial technical illustration of a large iron wireframe cube compressed into a compact burnt-orange monolith',
    markdown: goSizedMd,
  },
  {
    slug: 'introducing-djangors-1-0',
    title: 'Djangors 1.0: Batteries-Included Web Development in Rust',
    description:
      '38 crates published on crates.io with SemVer stability: ORM with Postgres/SQLite/MySQL & PostGIS, migrations, auto-admin, forms, auth with Django hash migration, REST, and OpenAPI.',
    date: '2026-11-22',
    readingTime: '7 min read',
    tags: ['Release', 'Djangors 1.0', 'Architecture'],
    cover: '/blog/djangors-1-0.png',
    coverAlt: 'Technical editorial illustration of modular charcoal and rust-orange industrial blocks snapping into one unified engine',
    markdown: djangors10Md,
  },
];

export function getPostBySlug(slug: string): BlogPost | undefined {
  return posts.find((post) => post.slug === slug);
}
