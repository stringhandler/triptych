// Copyright (c) 2024, The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

#![allow(missing_docs)]

#[macro_use]
extern crate criterion;
extern crate alloc;

use alloc::sync::Arc;

use criterion::Criterion;
use curve25519_dalek::{RistrettoPoint, Scalar};
use rand_chacha::ChaCha12Rng;
use rand_core::{CryptoRngCore, SeedableRng};
use triptych::{
    parameters::Parameters,
    proof::Proof,
    statement::{InputSet, Statement},
    witness::Witness,
};

// Parameters
const N_VALUES: [u32; 1] = [2];
const M_VALUES: [u32; 4] = [2, 4, 8, 10];
const BATCH_SIZES: [usize; 1] = [2];

// Generate a batch of witnesses and corresponding statements
#[allow(non_snake_case)]
fn generate_batch_data<R: CryptoRngCore>(
    params: &Arc<Parameters>,
    b: usize,
    rng: &mut R,
) -> (Vec<Witness>, Vec<Statement>) {
    // Generate witnesses; for this test, we use adjacent indexes for simplicity
    // This means the batch size must not exceed the input set size!
    assert!(b <= params.get_N() as usize);
    let mut witnesses = Vec::with_capacity(b);
    witnesses.push(Witness::random(params, rng));
    for _ in 1..b {
        let r = Scalar::random(rng);
        let l = (witnesses.last().unwrap().get_l() + 1) % params.get_N();
        witnesses.push(Witness::new(params, l, &r).unwrap());
    }

    // Generate input set from all witnesses
    let mut M = (0..params.get_N())
        .map(|_| RistrettoPoint::random(rng))
        .collect::<Vec<RistrettoPoint>>();
    for witness in &witnesses {
        M[witness.get_l() as usize] = witness.compute_verification_key();
    }
    let input_set = Arc::new(InputSet::new(&M));

    // Generate statements
    let mut statements = Vec::with_capacity(b);
    for witness in &witnesses {
        let J = witness.compute_linking_tag();
        let message = "Proof message".as_bytes();
        statements.push(Statement::new(params, &input_set, &J, Some(message)).unwrap());
    }

    (witnesses, statements)
}

#[allow(non_snake_case)]
#[allow(non_upper_case_globals)]
fn generate_proof(c: &mut Criterion) {
    let mut group = c.benchmark_group("generate_proof");
    let mut rng = ChaCha12Rng::seed_from_u64(8675309);

    for n in N_VALUES {
        for m in M_VALUES {
            // Generate parameters
            let params = Arc::new(Parameters::new(n, m).unwrap());

            let label = format!("Generate proof: n = {}, m = {} (N = {})", n, m, params.get_N());
            group.bench_function(&label, |b| {
                // Generate data
                let (witnesses, statements) = generate_batch_data(&params, 1, &mut rng);

                // Start the benchmark
                b.iter(|| {
                    // Generate the proof
                    let _proof = Proof::prove(&witnesses[0], &statements[0], &mut rng).unwrap();
                })
            });
        }
    }
    group.finish();
}

#[allow(non_snake_case)]
#[allow(non_upper_case_globals)]
fn generate_proof_vartime(c: &mut Criterion) {
    let mut group = c.benchmark_group("generate_proof_vartime");
    let mut rng = ChaCha12Rng::seed_from_u64(8675309);

    for n in N_VALUES {
        for m in M_VALUES {
            // Generate parameters
            let params = Arc::new(Parameters::new(n, m).unwrap());

            let label = format!(
                "Generate proof (variable time): n = {}, m = {} (N = {})",
                n,
                m,
                params.get_N()
            );
            group.bench_function(&label, |b| {
                // Generate data
                let (witnesses, statements) = generate_batch_data(&params, 1, &mut rng);

                // Start the benchmark
                b.iter(|| {
                    // Generate the proof
                    let _proof = Proof::prove_vartime(&witnesses[0], &statements[0], &mut rng).unwrap();
                })
            });
        }
    }
    group.finish();
}

#[allow(non_snake_case)]
#[allow(non_upper_case_globals)]
fn verify_proof(c: &mut Criterion) {
    let mut group = c.benchmark_group("verify_proof");
    let mut rng = ChaCha12Rng::seed_from_u64(8675309);

    for n in N_VALUES {
        for m in M_VALUES {
            // Generate parameters
            let params = Arc::new(Parameters::new(n, m).unwrap());

            let label = format!("Verify proof: n = {}, m = {} (N = {})", n, m, params.get_N());
            group.bench_function(&label, |b| {
                // Generate data
                let (witnesses, statements) = generate_batch_data(&params, 1, &mut rng);

                // Generate the proof
                let proof = Proof::prove(&witnesses[0], &statements[0], &mut rng).unwrap();

                // Start the benchmark
                b.iter(|| {
                    // Verify the proof
                    assert!(proof.verify(&statements[0]));
                })
            });
        }
    }
    group.finish();
}

#[allow(non_snake_case)]
#[allow(non_upper_case_globals)]
fn verify_batch_proof(c: &mut Criterion) {
    let mut group = c.benchmark_group("verify_batch_proof");
    let mut rng = ChaCha12Rng::seed_from_u64(8675309);

    for n in N_VALUES {
        for m in M_VALUES {
            // Generate parameters
            let params = Arc::new(Parameters::new(n, m).unwrap());

            for batch in BATCH_SIZES {
                let label = format!(
                    "Verify batch proof: n = {}, m = {} (N = {}), {}-batch",
                    n,
                    m,
                    params.get_N(),
                    batch
                );
                group.bench_function(&label, |b| {
                    // Generate data
                    let (witnesses, statements) = generate_batch_data(&params, 1, &mut rng);

                    // Generate the proofs
                    let proofs = witnesses
                        .iter()
                        .zip(statements.iter())
                        .map(|(w, s)| Proof::prove_vartime(w, s, &mut rng).unwrap())
                        .collect::<Vec<Proof>>();

                    // Start the benchmark
                    b.iter(|| {
                        // Verify the proofs in a batch
                        assert!(Proof::verify_batch(&statements, &proofs));
                    })
                });
            }
        }
    }
    group.finish();
}

criterion_group! {
    name = generate;
    config = Criterion::default();
    targets = generate_proof, generate_proof_vartime
}

criterion_group! {
    name = verify;
    config = Criterion::default();
    targets = verify_proof, verify_batch_proof
}

criterion_main!(generate, verify);
