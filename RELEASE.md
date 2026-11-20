# Releasing

A release raises the version, merges the changelog, and tags the merged commit. The tag starts the release workflow, which builds the packages and publishes the GitHub release.

## The release commit

On a `release/X.Y.Z` branch, in one commit:

1. Raise `[workspace.package] version` in `Cargo.toml` and the local default version in `apps/dreame-android/app/build.gradle.kts`. Run `cargo check` to update `Cargo.lock`.
2. Merge the changelog fragments:

   ```sh
   task changelog:release -- X.Y.Z
   ```

   This adds the `## [X.Y.Z] - <date>` section to `CHANGELOG.md` and deletes the files in `changelog/unreleased/`. Read the new section and fix its wording in `CHANGELOG.md` if needed. The command fails when there are no fragments; a release with nothing to tell users still needs one.

3. Run `task check`.

Change nothing else in this commit. The CI workflow recognizes a commit that changes only the version numbers and the changelog, and skips the build jobs for it, since `master` has already passed them and the release workflow builds everything again.

## Merging and tagging

`master` is protected, so open a pull request for the release branch and merge it once the checks pass. Then tag the merged commit with an annotated tag and push the tag:

```sh
git tag -a vX.Y.Z -m "X.Y.Z" <merged commit>
git push origin vX.Y.Z
```

The release workflow first checks that `CHANGELOG.md` has a section for the tag's version, and fails at once if not. The GitHub release notes are that section, followed by GitHub's list of merged pull requests.
