import { createHash } from 'node:crypto';

// Go gh transports the request; JavaScript parses and projects it independently.
const fields = new Set('id node_id pull_request_url pull_request_review_id in_reply_to_id commit_id original_commit_id path diff_hunk side line start_side start_line original_line original_start_line position original_position subject_type state submitted_at html_url url'.split(' '));
const digest = value => createHash('sha256').update(value, 'utf8').digest('hex');
function project(row) {
  const native = Object.fromEntries(Object.entries(row).filter(([key]) => fields.has(key)).map(([key, value]) => [key, key === 'diff_hunk' && value !== null ? digest(value) : value]));
  return { native, keys: Object.keys(row).sort(), nulls: Object.keys(row).filter(key => row[key] === null).sort(), bodySha256: typeof row.body === 'string' ? digest(row.body) : null };
}
async function get(path) {
  const child = Bun.spawn(['gh', 'api', '--hostname', 'github.com', '--method', 'GET', '--include', '-H', 'Accept: application/vnd.github+json', '-H', 'X-GitHub-Api-Version: 2022-11-28', path], { stdout: 'pipe', stderr: 'pipe' });
  const [text, err, exit] = await Promise.all([new Response(child.stdout).text(), new Response(child.stderr).text(), child.exited]);
  if (exit !== 0) throw new Error(`gh failed ${exit}: ${err}`);
  const separator = /\r?\n\r?\n/.exec(text);
  if (!separator) throw new Error('missing response headers');
  const headers = text.slice(0, separator.index).split(/\r?\n/);
  const status = Number(headers[0].split(' ')[1]);
  if (status !== 200) throw new Error(`unexpected status ${status}`);
  const link = headers.find(line => /^link:/i.test(line));
  return { value: JSON.parse(text.slice(separator.index + separator[0].length)), link: link ? link.slice(link.indexOf(':') + 1).trim() : null, status };
}
const result = {};
for (const family of ['reviews', 'comments']) {
  const base = `repos/rust-lang/rust/pulls/159232/${family}`;
  const all = await get(`${base}?per_page=100`);
  const first = await get(`${base}?per_page=1`);
  if (!all.value.length || !first.value.length) throw new Error('sample no longer populated');
  const id = first.value[0].id;
  const itemPath = family === 'reviews' ? `${base}/${id}` : `repos/rust-lang/rust/pulls/comments/${id}`;
  const item = await get(itemPath);
  result[family] = { all: all.value.map(project), first: project(first.value[0]), item: project(item.value), link100: all.link, link1: first.link, statuses: [all.status, first.status, item.status], itemPath };
}
await Bun.write(new URL('./oracle-result.json', import.meta.url), JSON.stringify(result, null, 2) + '\n');
console.log(JSON.stringify(Object.fromEntries(Object.entries(result).map(([key, row]) => [key, {count: row.all.length, statuses: row.statuses, nextLink: row.link1}])), null, 2));
