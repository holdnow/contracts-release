# Hold.now mainnet contract source

This repository contains the public source and pinned build recipe for the Hold.now Solana program.

Mainnet program: `BenqJ8z7KVZ3Y5JvrUAh7Z7koa5c91GsoqbT8kPjV7Xr`.
Deployment, initialization, and independent remote verification completed on November 16, 2026. Public access to Hold.now remains disabled.

Verified source: [`f099a9071d80a5eb08cbb349ef0bfb2c3029b419`](https://github.com/holdnow/contracts-release/tree/f099a9071d80a5eb08cbb349ef0bfb2c3029b419). [Official verification status](https://verify.osec.io/status/BenqJ8z7KVZ3Y5JvrUAh7Z7koa5c91GsoqbT8kPjV7Xr).
The deployed program contains embedded security.txt metadata linking to this repository’s private reporting channel and security policy.
The prior program `ETx7pCALkE4hruLUfrUte3VEt2NWyx2fmnxzNwZwApG9` has been permanently closed and must not be used.

## Reproduce the build

Install Docker and [Solana Verify 0.5.2](https://github.com/solana-foundation/solana-verifiable-build/releases/tag/v0.5.2), then run:

```sh
git checkout f099a9071d80a5eb08cbb349ef0bfb2c3029b419
solana-verify build --base-image solanafoundation/solana-verifiable-build@sha256:0b4e3716fad9ca4b4aac3e3f977f43aad93a18c22296c0c0f44fc22e644bdd68 --library-name holdnow --arch v3 '--cargo-build-sbf-args=--tools-version v1.58' -- --features default
solana-verify get-executable-hash target/deploy/holdnow.so
```

Use `default` for mainnet; do not enable `devnet`.

- Binary size: 610,256 bytes.
- SHA-256: `426702b534ab76f4771be0cc8aeb29402601f0584e3b4eae7d46f2b0cb5d4184`.
- Solana Verify hash: `7660f5c4f9d4ccd435ad659b474c55e5a97917cd42984c19990fa69ffcb4d2b0`.

Two independent official-container builds produced identical bytes. The exact published source commit was also rebuilt successfully and independently verified against the mainnet program. The contract suite passed 164 tests, including 20 integrations using pinned real Raydium/OpenBook programs. The program embeds security.txt reporting metadata. Its instructions, account layouts and launch economics are unchanged from the prior tested release.

The source inventory, container digest and build arguments are in `VERIFIABLE_BUILD.json`. The verification hash excludes trailing zero padding. Reproducibility does not replace a security audit.
The snapshot includes contract tests; the private release pipeline also uses external-program and backend/frontend fixtures that are not part of this minimal source repository.

## Security reporting

Please use [private vulnerability reporting](https://github.com/holdnow/contracts-release/security/advisories/new). See [SECURITY.md](SECURITY.md) and [security.txt](security.txt).

See [Solana's Verified Builds instructions](https://solana.com/docs/programs/verified-builds) for on-chain metadata and independent remote verification.
