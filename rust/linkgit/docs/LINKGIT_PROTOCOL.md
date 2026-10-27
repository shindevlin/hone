# LinkGit Protocol

LinkGit is a custom Git remote helper that stores repositories on the BTCPC chain.

## URL Format

```
linkgit://owner/repo
```

## Storage Model

Git objects (blobs, trees, commits, tags) are stored as BTCPC chain transactions. Each object is identified by its SHA-1 hash. Refs (branches, tags) are stored as named pointers to object hashes.

## Mirroring

LinkGit supports simultaneous push to multiple remotes via `.linkgit/mirrors`, a config file tracked inside the repo itself so mirror config travels with every clone.

### Setup

```bash
# Add a GitHub mirror
linkgit mirror add github https://github.com/owner/repo

# Wire up git push URLs (run after clone)
linkgit mirror apply

# Now one push goes everywhere
git push origin main

# Or sync manually to all mirrors
linkgit mirror sync
```

### .linkgit/mirrors format

```toml
[mirror.github]
url = "https://github.com/owner/repo"
push = true    # include in git push
fetch = false  # don't fetch from this mirror
```

### Commands

| Command | What it does |
|---------|-------------|
| `linkgit mirror add <name> <url>` | Add a mirror |
| `linkgit mirror remove <name>` | Remove a mirror |
| `linkgit mirror list` | Show configured mirrors |
| `linkgit mirror apply [remote]` | Wire git push URLs from .linkgit/mirrors |
| `linkgit mirror sync` | Push current branch to all mirrors |
