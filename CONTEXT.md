# ResourceFS

ResourceFS is a standalone product inspired by Oh My Pi’s path-shaped tools. It gives agent harnesses a uniform way to address and operate on workspace and external information without depending on Oh My Pi.

## Language

**ResourceFS**:
A standalone cross-harness Path Resource Server that exposes path-shaped resource operations to agent harnesses through MCP.
_Avoid_: MCP version of OMP, OMP remote surface, OMP compatibility wrapper

**Path Reference**:
A path-shaped, server-relative name for a resource or a projection of one; it does not imply that the resource is backed by a local file or globally owns its URI scheme.
_Avoid_: File path when referring to virtual, remote, or generated resources

**Resource**:
Information addressable by a Path Reference, including workspace content, remote objects, generated output, and session-owned artifacts.
_Avoid_: File when the backing form is not necessarily a filesystem object

**Workspace Root**:
A client-declared or launch-configured directory that bounds the filesystem resources accessible to a Path Resource Server.
_Avoid_: Process working directory when describing access authority

**Portable Source**:
A resource family whose state and credentials can be owned by the Path Resource Server without access to an agent harness's internal runtime.
_Avoid_: OMP parity

**Atlassian Site Mount**:
A configured Portable Source child that binds one Server Profile-unique lowercase Site ID to one Atlassian Cloud tenant and one credentialed visibility graph, independently enabling Jira and Confluence resources.
_Avoid_: Atlassian Site when referring to configured authority, Atlassian account, tenant URL

**Database Mount**:
A configured Portable Source instance that binds one Server Profile-unique lowercase Mount ID to one database provider and one logical database, addressed through provider-neutral `db://` Path References for the lifetime of that ResourceFS process.
_Avoid_: Database file, database server, connection string, implicit SQLite database, globally resolvable database URI

**Database Value**:
A lossless provider-neutral JSON scalar or recognized tagged extension for one authoritative database cell, used consistently in canonical row content and primary-key identity.
_Avoid_: Provider-formatted text fallback, inferred SQLite affinity value, generic JSON number for every numeric type

**Path Session**:
The state owned by one MCP connection, including artifacts, scratch resources, edit snapshots, and caches; its references expire when the connection closes, even if storage is retained briefly for cleanup or diagnosis.
_Avoid_: Conversation, workspace session

**Server Profile**:
The explicit source configuration and credentials used by a Path Resource Server independently of an agent harness's internal configuration.
_Avoid_: OMP config

**Launch Authority**:
The one operator-selected source of initial Workspace Root authority for a ResourceFS process: either one Server Profile or one or more CLI Workspace Roots. Client-declared MCP Roots may replace that authority for a connection; the process working directory never supplies it implicitly.
_Avoid_: Default workspace, current-directory fallback

**Recovery Reference**:
A Path Reference returned with bounded output that addresses content omitted from the result.
_Avoid_: Truncation notice when the omitted content remains recoverable

**Atlassian Query Resource**:
A read-only collection Resource scoped to one Atlassian Site Mount and one product (Jira or Confluence), whose address carries that product's exact native JQL or CQL and whose bounded result rows navigate to canonical stable-ID Resources.
_Avoid_: ResourceFS query language, cross-site search, search snapshot

**Primary Workspace Root**:
The Workspace Root under which relative filesystem references resolve; every tool addresses resources in other declared roots with canonical `rfs://workspace/<root>/<path>` references.
_Avoid_: Searching every root for a relative path

**Session Scratch**:
Caller-authored `local://` resources contained within a Path Session and writable without Workspace Root mutation permission, subject to object and session quotas.
_Avoid_: Workspace file, temporary workspace

**Workspace Mutation**:
A create, update, or deletion of a resource contained by a Workspace Root; it excludes writes to Session Scratch.
_Avoid_: Any write when referring to server-owned scratch state

**Memory Resource**:
A read-only `memory://` projection of project memory supplied by a configured source.
_Avoid_: Memory command, mutable memory document

**Mutation Policy**:
The grants attached to a Workspace Root or configured Portable Source that authorize creates, updates, and deletions independently of an agent harness's approval UI.
_Avoid_: Allow-write flag, client approval

**Structural Summary**:
A bounded view of code in the supported language registry that exposes declarations while eliding bodies and names the Path References needed to recover omitted ranges.
_Avoid_: File contents, truncation

**Behavior Contract**:
ResourceFS's own versioned reference, read, search, mutation, and recovery semantics; OMP is design input rather than a continuing compatibility authority.
_Avoid_: OMP parity, OMP conformance

**Source Mutation**:
A change to a non-workspace Resource, authorized by the Mutation Policy of its configured Portable Source.
_Avoid_: Workspace Mutation, adapter enabled

**Version Tag**:
A content-derived identifier in a read header that names the exact Resource state against which an edit is valid.
_Avoid_: Line number, modification time

**Aggregate Resource**:
A read-only Resource that combines several authoritative upstream values into one navigable representation.
_Avoid_: Document when writing it would imply replacing every rendered field

**Field Resource**:
A Resource that corresponds to one authoritative upstream value and may be mutated when its source grants that operation.
_Avoid_: Aggregate field, property path

**Creation Target**:
A Path Reference ending in `/new` that accepts a create through `rfs_write` and returns the created Resource's canonical Path Reference.
_Avoid_: Mutable collection, temporary Resource

**Agent Export**:
A strict versioned manifest and contained files that project a foreign harness's agent outputs and transcripts through `agent://` and `history://`.
_Avoid_: OMP session directory, transcript database

**Resource Mirror**:
The bounded MCP Resources/templates projection of the same address space exposed canonically through `rfs_*` tools.
_Avoid_: Canonical interface, tool replacement

**Degraded Source**:
An optional configured source that is unavailable while ResourceFS remains active; only references owned by that source fail, and its state is visible in diagnostics.
_Avoid_: Ignored startup failure

**Source Adapter**:
The internal module that implements one Resource family's resolution, projection, mutation, versioning, listing, and policy capabilities behind the shared engine.
_Avoid_: MCP server, plugin

**Backing-Path Visibility**:
A Server Profile option that adds backing `file://` display and metadata beside a workspace Resource's canonical `rfs://` identity; it never replaces that identity.
_Avoid_: File URI mode, canonical-path switch
