#!/usr/bin/env python3
"""Compare production facts JSON with an independently supplied native response.
Usage: python facts_consumer.py PRODUCTION.json NATIVE.json [OBSERVATIONS.json]
Reads '-' as production stdin. Unknown additive fields are deliberately ignored.
"""
import json
import sys


def compare(facts, native, observations=None):
    if facts['schemaVersion']['major'] != 1:
        raise ValueError('unsupported schema major')
    assert facts['kind'] == 'github.pull_request'
    assert facts['resource'] == 'pr://owner/repo/7/facts'
    assert facts['request'] == {'repository': {'owner': 'owner', 'name': 'repo'}, 'number': '7'}
    data = facts['data']
    def presence(source, target, old, new, convert=lambda x: x):
        assert (old in source) == (new in target), (old, 'presence')
        if old in source:
            expected = None if source[old] is None else convert(source[old])
            assert target[new] == expected, (old, target[new], expected)
    for old, new in [('id','id'), ('number','number')]:
        presence(native, data, old, new, str)
        assert facts['observed'][new] == str(native[old])
    for old, new in [('node_id','nodeId'), ('title','title'), ('body','body'), ('state','state'), ('draft','draft'), ('merged','merged'), ('created_at','createdAt'), ('updated_at','updatedAt'), ('closed_at','closedAt'), ('merged_at','mergedAt')]:
        presence(native, data, old, new)
    for side in ['base', 'head']:
        src, dst = native[side], data[side]
        assert dst['commitSha'] == src['sha'] == facts['observed'][side]['commitSha']
        presence(src, dst, 'ref', 'refName')
        assert dst['repositoryAvailability'] == ('omitted' if 'repo' not in src else 'null' if src['repo'] is None else 'present')
        assert ('repo' in src) == ('repository' in dst)
        if src.get('repo') is None:
            if 'repo' in src:
                assert dst['repository'] is None
        else:
            for old, new in [('id','id'), ('node_id','nodeId'), ('name','name'), ('full_name','fullName')]:
                presence(src['repo'], dst['repository'], old, new, str if old == 'id' else lambda x: x)
    assert ('user' in native) == ('author' in data)
    if native.get('user') is None:
        if 'user' in native:
            assert data['author'] is None
    else:
        for old, new in [('id','id'), ('node_id','nodeId'), ('login','login')]:
            presence(native['user'], data['author'], old, new, str if old == 'id' else lambda x: x)
    for old, new in [('url','apiUrl'), ('html_url','htmlUrl'), ('diff_url','diffUrl'), ('patch_url','patchUrl'), ('issue_url','issueUrl'), ('_links','relations')]:
        presence(native, data['links'], old, new)
    assert facts['acquisition']['restApiVersion'] == '2022-11-28'
    assert facts['acquisition']['startedAtUnixMs'] <= facts['acquisition']['completedAtUnixMs']
    assert 'versionTag' not in facts and 'finalBytes' not in facts['acquisition']['usage']
    if observations:
        for key, expected in observations.items():
            actual = facts
            for part in key.split('.'):
                actual = actual[part]
            assert actual == expected, (key, actual, expected)


if __name__ == '__main__':
    production = json.load(sys.stdin if sys.argv[1] == '-' else open(sys.argv[1], encoding='utf-8'))
    native = json.load(open(sys.argv[2], encoding='utf-8'))
    observations = json.load(open(sys.argv[3], encoding='utf-8')) if len(sys.argv) > 3 else None
    compare(production, native, observations)
    additive = dict(production, futureOptional={'opaque': True})
    compare(additive, native, observations)
    print('native facts comparison and additive compatibility passed')
