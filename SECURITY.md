# Hold.now security policy

## Reporting a vulnerability

Please report suspected vulnerabilities privately using
[GitHub's Report a vulnerability form](https://github.com/holdnow/contracts-release/security/advisories/new).
Do not disclose exploit details in public issues, pull requests, or discussions.

Include the affected program address and network, source commit or release,
expected and observed behavior, potential impact, and reproduction steps or a
minimal proof of concept. Never include wallet private keys, recovery phrases,
API credentials, or other users' personal data.

## Scope

This policy covers the Hold.now contract source published in this repository and
the mainnet program `ETx7pCALkE4hruLUfrUte3VEt2NWyx2fmnxzNwZwApG9`.
The README identifies the deployed, verified source revision; later repository
commits do not necessarily represent deployed code. Reports affecting that
revision or newer published contract source are welcome.

## Coordinated disclosure

Please allow time for investigation and a fix before publishing vulnerability
details. Use a local validator or devnet for reproduction; do not access other
users' data, move their funds, or disrupt services. This policy does not authorize
testing against third-party systems or promise a bounty or response deadline.

The repository's `security.txt` points to this reporting policy. It is not embedded
in the currently deployed program and does not change its verified binary.
