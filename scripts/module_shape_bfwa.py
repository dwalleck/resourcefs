#!/usr/bin/env python3
"""C02 staged successor to the shared placement census for this issue.

Loads the sibling `module_shape.py` (the published successor of the shared
machinery) and runs it against this issue's merged ledger. The ledger is the
only policy input; the default stage lives in the ledger and --stage overrides
it for an explicit checkpoint. --root is an independent disposable source tree.
"""
import argparse
import importlib.util
import json
from pathlib import Path
import sys

# Importing the predecessor oracle must not leave generated caches in the tree.
sys.dont_write_bytecode = True

HERE = Path(__file__).resolve().parent


def load_predecessor():
    spec = importlib.util.spec_from_file_location("bfwa_shape", HERE / "module_shape.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--stage', help='explicit checkpoint stage; defaults to the ledger stage')
    parser.add_argument('--root', type=Path, help='disposable source tree')
    parser.add_argument('--repository', type=Path, default=HERE.parent,
                        help='Git checkout providing the pinned baseline')
    args = parser.parse_args()
    try:
        repository = args.repository.resolve(strict=True)
        predecessor = load_predecessor()
        inherited = predecessor.load_inherited()
        repository = Path(inherited.git(repository, 'rev-parse', '--show-toplevel'))
        root = args.root.resolve(strict=True) if args.root else repository
        # Policy is trusted checker input, not mutable fixture/source content.
        ledger = json.loads((HERE / 'module-ledger-bfwa.json').read_text(encoding='utf-8'))
        stage = args.stage or ledger['stage']
        if stage not in ledger['stages']:
            raise ValueError(f'invalid explicit policy stage {stage!r}')
        failures, observations = predecessor.check(root, repository, ledger, stage)
    except (OSError, UnicodeError, ValueError, ImportError, KeyError) as error:
        print(f'C02 FAIL oracle input: {error}', file=sys.stderr)
        return 1
    print('\n'.join(observations))
    if failures:
        print('\n'.join(failures), file=sys.stderr)
        return 1
    print(f'C02 PASS stage={stage} baseline={ledger["baseline"]}')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
