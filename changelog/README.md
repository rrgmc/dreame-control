# Changelog fragments

Each pull request that changes something a user would notice adds one file to `changelog/unreleased/`. A release merges all of them into [CHANGELOG.md](../CHANGELOG.md) and deletes them. Separate files mean two pull requests never edit the same lines, so they do not conflict.

## Writing a fragment

Name the file after the branch or the change, for example `changelog/unreleased/schedule-delete.md`. The name is not published; it only orders the entries within a section.

Use the [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) sections as `###` headings, each followed by list items:

```markdown
### Fixed

- Deleting a schedule now removes it from the robot.

### Added

- The `dreame schedules` command lists the robot's schedules.
```

The sections are `Added`, `Changed`, `Deprecated`, `Removed`, `Fixed` and `Security`. Use only the ones you need. Write for the people who use the apps: say what changed for them, not how the code changed.

Changes that users do not notice, such as refactoring, tests or CI, need no fragment.

Check the fragments with:

```sh
task changelog:check
```

## Releasing

The release commit runs:

```sh
task changelog:release -- 1.7.0
```

This adds a `## [1.7.0] - <today>` section to the top of `CHANGELOG.md`, groups the entries of all fragments under their sections, and deletes the fragments. The release workflow then uses that section as the GitHub release notes. See [RELEASE.md](../RELEASE.md).
