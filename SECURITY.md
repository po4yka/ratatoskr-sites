# Security Policy for Ratatoskr Sites

Report vulnerabilities privately. Do not publish vault content, the content of an unpublished note,
database credentials, nkey seeds, or exploit vaults.

Security review is mandatory for: the selection of a publication, the privacy filter of the release
builder, the HTML allowlist, the reads from the notes blob root and their verification, the
manifest and the effective capability set, the claims that Edge mints, and every route that answers
an anonymous caller.

Baseline: a release holds no data read from a file that its publication does not select; the
privacy filter runs once on the parsed tree before any renderer; note bytes are read only through
the read group `ratatoskr-notes-blobs`, are verified against their `BlobRef` before use, and nothing
is written to the notes blob root; the domain API is loopback-only and trusts only the claims that
Edge mints; an unknown, unapproved or withdrawn site gives the same answer as a site that does not
exist.
