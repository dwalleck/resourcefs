# Evidence: rfs-cgbq

## Premise checklist

| ID | Candidate premise | Smallest question | Verdict |
|----|-------------------|-------------------|---------|
| P1 | `rmcp` 3.1.3 can expose client Roots under MCP `2026-07-28`, but the legal server-request scope must be established empirically. | In one real 2026-07-28 rmcp connection, do `roots/list` calls from initialization/change notification handlers and from an incoming request handler succeed or fail, and does the request-scoped call retrieve the initial and changed sets? | PASS — learning: SEP-2260 rejects notification-scoped calls; request-scoped calls succeed |
| P2 | A platform path API can supply the specification's Windows drive/UNC classification without an explicit grammar. | Does Win32 `PathIsRelativeA` classify drive-absolute, UNC, drive-relative, root-relative, and slash-rooted forms identically to Rust's fully-qualified Windows `Path::is_absolute` contract? | PASS — learning: no; explicit classification is required |
| P3 | Windows handle-final canonicalization exposes a reparse/symlink escape in a component-comparable form. | When a directory link below a root targets an outside directory, does handle-final canonicalization resolve the target outside the root and return normalized verbatim path form? | PASS |
| P4 | Kiro itself must supply MCP Roots for this design to work. | Does any behavior in `spec.md` require Kiro-specific Roots support? | N/A — the contract is generic MCP; the complete real-Kiro gate is verified open issue rfs-5os7 |
| P5 | File-URI, percent-decoding, selector, identity, limits, and invalidation outcomes require probing an existing implementation. | Are these claims about an existing system? | N/A — requester-approved `spec.md` defines new feature behavior; design and checkpoints must falsify it |

## Data

- Source: production-shaped.
- Shape: a pinned `rmcp = 3.1.3` client/server handshake explicitly negotiating `2026-07-28` over Tokio duplex, with the real Roots capability, notification-scoped and request-scoped `roots/list` attempts, initialized, and `notifications/roots/list_changed` flow; a temporary Win32-shaped fixture containing drive/UNC strings and a directory symbolic link from a declared root to an outside directory.
- Safety: both probes create only temporary processes, crates, directories, and files; they do not read, write, mutate, or delete repository production state. The Windows probe uses Wine's Win32 APIs for runtime observation; the independent oracle reads the installed Rust Windows standard-library implementation and creates a separate POSIX realpath fixture.

## Probe

- Files: `probe_rmcp.py`, `probe_windows.py`.
- Mechanism: `probe_rmcp.py` compiles a throwaway Rust program against the pinned real SDK, starts client and server services over an in-memory transport, explicitly negotiates `2026-07-28`, attempts `roots/list` from initialized/change notification handlers, then repeats inside `tools/list` request-handler scope and records both root sets plus SEP-2260 rejection. `probe_windows.py` compiles a throwaway Win32 C program with `winegcc`, calls `PathIsRelativeA`, creates a directory link through Win32 `mklink`, and calls `GetFinalPathNameByHandleA` on the root and escaped target.
- Run: `python3 .rfs-cgbq/probe_rmcp.py`
- Run: `python3 .rfs-cgbq/probe_windows.py`

## Oracle

- Mechanism: `oracle.py` statically inspects independent installed `rmcp` 3.1.3 dispatch/model source and Rust 1.94.0 Windows path/filesystem source, computes the lexical table with Python `PureWindowsPath`, and verifies escape topology with a separately created `realpath` fixture. Static dispatch/source inspection and Python path resolution fail differently from the runtime rmcp handshake, Win32/Wine calls, and the future production implementation.
- Run: `python3 .rfs-cgbq/oracle.py`

## Comparisons

| ID | Probe output | Oracle output | Verdict |
|----|--------------|---------------|---------|
| P1 | Under explicit `2026-07-28`, initialized/change notification attempts reported `associationRejected=true` and never reached the client; `tools/list` request-scoped attempts returned `[file:///workspace/alpha]` and then `[file:///workspace/beta, file:///workspace/gamma]`; `listRootsCalls=2`. | The pinned source restricts `ListRootsRequest` under SEP-2260, scopes `ORIGINATING_REQUEST` around request handlers but not notification handlers, and contains the Roots capability/dispatch APIs; the hand-authored four-event lifecycle and call count exactly match. | PASS — direct notification acquisition premise disproved; request-associated acquisition validated |
| P2 | Win32/Wine reported drive absolute `true`, UNC absolute `true`, drive-relative `true`, root-relative `true`, slash-rooted `false`. | Rust's Windows contract requires both a prefix and a root; the independent table reports drive absolute `true`, UNC absolute `true`, drive-relative `false`, root-relative `false`, slash-rooted `false`. Inspection confirmed Win32 `PathIsRelativeA` is not equivalent to Rust's fully-qualified absolute contract for drive-relative and root-relative inputs. | PASS — disagreement resolved as a model correction |
| P3 | The Win32-created escaping link produced `escapeStartsWithRoot=false`; handle-final root output used the `\\?\` verbatim prefix. | Rust Windows `canonicalize` opens without `FILE_FLAG_OPEN_REPARSE_POINT`, obtains its result with `GetFinalPathNameByHandleW`, and the independent fixture resolves the same topology outside its root. | PASS |
| P4 | N/A — no probe. | N/A — `spec.md` and rfs-5os7 establish the boundary. | N/A — non-premise |
| P5 | N/A — no probe. | N/A — `spec.md` owns these outcomes. | N/A — non-premise |

## Validated / learned

- P1: learned and validated — rmcp 3.1.3 delivers initial and changed Roots under negotiated MCP `2026-07-28` only when `roots/list` is issued inside an incoming request-handler scope. SEP-2260 rejects the same call from `on_initialized`, `on_roots_list_changed`, or a spawned task because those scopes lack `ORIGINATING_REQUEST`. The SDK also marks Roots deprecated by SEP-2577 and its generated `list_roots()` uses `PeerRequestOptions::no_options()`. ResourceFS must suspend authority in notification handlers, park the refresh, acquire inline during the next incoming request with an explicit total 5-second deadline, and keep all rmcp types in the MCP adapter.
- P2: learning — Win32 `PathIsRelativeA` is not a valid oracle for the requested fully-qualified Windows grammar: it classified `C:workspace\\file.txt` and `\\workspace\\file.txt` as non-relative, while Rust requires both prefix and root. The feature must not delegate the Behavior Contract to that API.
- P3: validated prior understanding — handle-final canonicalization follows the Win32-created directory link, exposes the outside target, and uses normalized verbatim form; component-aware comparison must account for that prefix and cannot rely on lexical input spelling.

## Related issues

- Consulted: rfs-jk0d (accepted parent Behavior Contract); rfs-87cv (founding single-root implementation and prior rmcp evidence); rfs-r9m6 (strict profile wiring); rfs-34pz (selector execution and Path Session state); rfs-73dz (mutation snapshots); rfs-ww6w (MCP Resource mirroring and notifications); rfs-58r1 (release platform matrix); rfs-5os7 (real-Kiro and release evidence).
- Filed: none — P1-P3 reached PASS; P4 and P5 are covered non-premises, and every deferred behavior already has the verified issue named above.
