# AUR publication

1. Create the AUR package `clove` once from your account (empty repo is fine).
2. Add your AUR SSH private key as the repository secret `AUR_SSH_PRIVATE_KEY`.
3. Push a version tag, e.g. `git tag v0.1.0 && git push origin v0.1.0`.
4. The `Publish AUR` workflow generates `.SRCINFO` and pushes `PKGBUILD` + `.SRCINFO`.

The package builds Clove from the GitHub release source tarball plus the pinned Rune
checkout, so the release tag must exist before AUR builds it.
