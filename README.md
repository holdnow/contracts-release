# Hold.now mainnet contract source

This repository contains a clean source snapshot for independently rebuilding the
Hold.now Solana program. It has no private repository history or deployment keys.

**Mainnet deployment is paused.** This source has not yet received Solana Verified
Builds status. Reproducibility does not constitute a security audit.

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

After deployment is authorized, verify against this repository's exact public
commit with the same image, architecture and build arguments, compare the on-chain
binary, upload verification metadata using the program's upgrade authority, and
submit an independent remote verification job. No deployment keys belong in this
repository or in a public build job.

See [Solana's verified-build instructions](https://solana.com/docs/programs/verified-builds).
