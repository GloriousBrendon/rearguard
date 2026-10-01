# Security policy

Rearguard is an anti-cheat, so two kinds of report belong here:

- **Bypasses:** a way for a cheat to avoid detection, to learn a secret probe signal or
  seed it should not have, or to get an honest player flagged.
- **Vulnerabilities:** anything in the server, the wire protocol, the Godot extension or
  the study build that harms whoever runs it, such as a crash from untrusted telemetry,
  a leaked secret or personal data in a study export.

## How to report

Report privately, through GitHub's private vulnerability reporting: on the repository's
**Security** tab, choose **Report a vulnerability**.

Please do not open a public issue, pull request or discussion for a bypass or a
vulnerability, and do not post the details anywhere public before it is fixed.

Private vulnerability reporting has to be enabled by a maintainer in the repository
settings (Settings, Security, "Private vulnerability reporting"). If the
**Report a vulnerability** button is missing, it has not been enabled: open a public
issue that says only that you have a security report and asks for it to be enabled, with
no details.

No e-mail address is published for reports.

## What to include

- What the cheat or attacker does, and what they gain.
- The revision (commit) you tested.
- Steps to reproduce it. A test cheat that runs inside Rearguard's own test environment
  (`rearguard-sim`, or the aim range's test bots) is the most useful form.

Test only against your own copy. Do not aim anything at other people's servers or games.

## What to expect

Rearguard is an early prototype maintained in spare time. Reports are read and answered
as time allows, with no guaranteed response time. No bounty or other reward is offered.

## Known limits

The limits listed under "Status and limits" in [README.md](README.md) are known and need
no private report: ordinary issues and pull requests are welcome for those.
