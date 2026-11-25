import type { APIRoute } from 'astro';
import { posts, type BlogPost } from '../../blog/posts';

export function getStaticPaths() {
  return posts.map((post) => ({
    params: { slug: post.slug },
    props: { post },
  }));
}

export const GET: APIRoute = ({ props }) => {
  const { post } = props as { post: BlogPost };
  return new Response(post.markdown, {
    headers: { 'content-type': 'text/markdown; charset=utf-8' },
  });
};
