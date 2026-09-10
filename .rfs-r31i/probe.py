import hashlib
import json
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent
FIELDS = 'id node_id pull_request_url pull_request_review_id in_reply_to_id commit_id original_commit_id path diff_hunk side line start_side start_line original_line original_start_line position original_position subject_type state submitted_at html_url url'.split()

class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        raise RuntimeError('redirect refused')

opener = urllib.request.build_opener(NoRedirect)

def get(path):
    url = 'https://api.github.com/' + path
    request = urllib.request.Request(url, headers={'Accept': 'application/vnd.github+json', 'X-GitHub-Api-Version': '2022-11-28', 'User-Agent': 'resourcefs-r31i-readonly-probe'})
    with opener.open(request, timeout=20) as response:
        body = response.read(8 * 1024 * 1024 + 1)
        if len(body) > 8 * 1024 * 1024:
            raise RuntimeError('response too large')
        return json.loads(body), response.headers.get('Link'), response.status

def project(row):
    values = {key: row[key] for key in FIELDS if key in row}
    if 'diff_hunk' in values:
        values['diff_hunk'] = hashlib.sha256(values['diff_hunk'].encode()).hexdigest() if values['diff_hunk'] is not None else None
    return {'native': values, 'keys': sorted(row), 'nulls': sorted(k for k, v in row.items() if v is None), 'bodySha256': hashlib.sha256(row['body'].encode()).hexdigest() if isinstance(row.get('body'), str) else None}

result = {}
for family in ['reviews', 'comments']:
    base = 'repos/rust-lang/rust/pulls/159232/' + family
    rows, link, status = get(base + '?per_page=100')
    first, first_link, first_status = get(base + '?per_page=1')
    if not rows or not first:
        raise RuntimeError('public sample no longer contains records')
    item_path = base + '/' + str(first[0]['id']) if family == 'reviews' else 'repos/rust-lang/rust/pulls/comments/' + str(first[0]['id'])
    item, item_link, item_status = get(item_path)
    result[family] = {'all': [project(row) for row in rows], 'first': project(first[0]), 'item': project(item), 'link100': link, 'link1': first_link, 'statuses': [status, first_status, item_status], 'itemPath': item_path}
(ROOT / 'probe-result.json').write_text(json.dumps(result, indent=2) + '\n')
print(json.dumps({key: {'count': len(value['all']), 'statuses': value['statuses'], 'listItemEqual': value['first'] == value['item'], 'nextLink': value['link1']} for key, value in result.items()}, indent=2))
