This PR template helps ensure that as we launch new features we appropriately communicate, track, document, make them accessible, etc.

## PRD checklist

- [ ] Plan for how to measure success quantified by metrics

## Coding checklist

- [ ] Test in dev for a week
- [ ] Telemetry in code
- [ ] A11y (if applicable, see [testing a11y guide](https://docs.google.com/document/d/1-H0bWss5Qw18ZpIYg-RUvN7_db1MVdWfOb5UF_GLxNc/edit?usp=sharing) for more info)
- [ ] Add to Command Palette (if applicable)
- [ ] Add toggle setting(s) to command palette (if applicable)
- [ ] Add to Mac Menu (if applicable)
- [ ] Add keybinding (if applicable), see [actions audit for inspiration](https://docs.google.com/spreadsheets/d/1C56ZIqDGjJi873-HAPdnT2DofC3Z6G-aJMYeQeERHx4/edit#gid=0)
- [ ] Sanity check within the app that it does not clash other keybindings
- [ ] No sensitive info in logs
- [ ] No crashes on dev related to the feature
- [ ] No performance regression in an InfiniShell-Desktop development build
- [ ] Feature works correctly, without regression, over SSH
- [ ] Have we explicitly brainstormed how this feature will be discovered by developers?
- [ ] Link to Figma mocks
- [ ] Tested on multiple themes (both dark and light)
- [ ] If the feature being released relies on a server API, that API has been stable on production for at least one full server release cycle


## Content checklist

- [ ] Help content
- [ ] Changelog entry (add entry below)
- [ ] Telemetry entry (if applicable)
- [ ] Metrics dashboard in Metabase
- [ ] Tweet (if appropriate)
- [ ] Blog post (if appropriate)

## Changelog

CHANGELOG-NEW-FEATURE: {{Insert a changelog entry here}}
