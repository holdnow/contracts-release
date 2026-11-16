# Hold.now mainnet contract source

This repository contains a clean source snapshot for independently rebuilding the
Hold.now Solana program. It has no private repository history or deployment keys.

**Deployed to mainnet and independently verified.** The remote verifier confirmed
that the deployed program matches public source commit
[`8852c95aac20a5ca94e56c35d1a49e87c14bd8a4`](https://github.com/holdnow/contracts-release/tree/8852c95aac20a5ca94e56c35d1a49e87c14bd8a4).
The public Hold.now application remains disabled pending launch readiness.
Reproducibility does not constitute a security audit.

## Build

Install Docker and [Solana Verify 0.5.2](https://github.com/solana-foundation/solana-verifiable-build/releases/tag/v0.5.2).
The verifier executable checksum, immutable container image, source-file inventory
and build arguments are recorded in `VERIFIABLE_BUILD.json`.

Run from this repository's root:

```sh
solana-verify build --base-image solanafoundation/solana-verifiable-build@sha256:0b4e3716fad9ca4b4aac3e3f977f43aad93a18c22296c0c0f44fc22e644bdd68 --library-name holdnow --arch v3 '--cargo-build-sbf-args=--tools-version v1.58' -- --features default
solana-verify get-executable-hash target/deploy/holdnow.so
```

Use the `default` feature for mainnet. Do not enable `devnet`.

## Expected output

- Program ID: `ETx7pCALkE4hruLUfrUte3VEt2NWyx2fmnxzNwZwApG9`
- ELF size: 609,888 bytes.
- Full-file SHA-256: `5e409b7f8a461ad24c0d88752472afb943a0ec6097b02f7f4857cb980b8e843e`
- Solana Verify hash: `88d8f382099da2811a1c85ffad4d30230e12a7a650162c44841b2d5bb74a00a7`

The hashes differ because Solana Verify excludes trailing zero padding. Two
independent container builds reproduced this exact binary. The mainnet contract
suite passed 163 tests against it, including 20 integration tests using pinned real
Raydium/OpenBook programs.

The snapshot retains the program's test sources. The release pipeline also uses
external-program fixtures and backend/frontend compatibility fixtures; those are
not included in this minimal build-source repository.

## Public verification

- [Official remote verification status](https://verify.osec.io/status/ETx7pCALkE4hruLUfrUte3VEt2NWyx2fmnxzNwZwApG9)
- [Mainnet program](https://explorer.solana.com/address/ETx7pCALkE4hruLUfrUte3VEt2NWyx2fmnxzNwZwApG9)
- Verification uploader / upgrade authority: `3jErXTgKy1g2FM6DovmxXLDfUtApYDPhUoiszptGwQVs`

Verification metadata is signed by the program's upgrade authority and pins the
exact source commit, immutable Docker image, architecture and build arguments above.
The on-chain and independently rebuilt program hashes match the expected Solana
Verify hash. Later documentation commits do not change that verified source revision.

To reproduce the verification, check out the pinned commit and use the recorded
build command. No deployment keys belong in this repository or a public build job.

See [Solana's verified-build instructions](https://solana.com/docs/programs/verified-builds).

## Reporting security issues

See [our security policy](SECURITY.md) for private vulnerability reporting.
