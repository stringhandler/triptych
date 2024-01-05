// Copyright (c) 2024, The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use alloc::vec::Vec;
use core::iter::once;

use curve25519_dalek::{
    traits::{Identity, MultiscalarMul, VartimeMultiscalarMul},
    RistrettoPoint,
    Scalar,
};
use merlin::Transcript;
use rand_core::CryptoRngCore;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
use snafu::prelude::*;
use zeroize::Zeroizing;

use crate::{statement::Statement, witness::Witness};

// Proof version flag
const VERSION: u64 = 0;

/// A Triptych proof.
#[allow(non_snake_case)]
#[cfg_attr(feature = "serde", derive(Deserialize, Serialize))]
#[derive(Clone, Eq, PartialEq)]
pub struct Proof {
    A: RistrettoPoint,
    B: RistrettoPoint,
    C: RistrettoPoint,
    D: RistrettoPoint,
    X: Vec<RistrettoPoint>,
    Y: Vec<RistrettoPoint>,
    f: Vec<Vec<Scalar>>,
    z_A: Scalar,
    z_C: Scalar,
    z: Scalar,
}

/// Errors that can arise relating to proofs.
#[derive(Debug, Snafu)]
pub enum ProofError {
    /// An invalid parameter was provided.
    #[snafu(display("An invalid parameter was provided"))]
    InvalidParameter,
    /// A transcript challenge was invalid.
    #[snafu(display("A transcript challenge was invalid"))]
    InvalidChallenge,
}

/// Kronecker delta function with scalar output.
fn delta(x: u32, y: u32) -> Scalar {
    if x == y {
        Scalar::ONE
    } else {
        Scalar::ZERO
    }
}

/// Get nonzero powers of a challenge value from a transcript.
///
/// If successful, returns powers of the challenge with exponents `[0, m]`.
/// If any power is zero, returns an error.
fn xi_powers(transcript: &mut Transcript, m: u32) -> Result<Vec<Scalar>, ProofError> {
    // Get the verifier challenge using wide reduction
    let mut xi_bytes = [0u8; 64];
    transcript.challenge_bytes("xi".as_bytes(), &mut xi_bytes);
    let xi = Scalar::from_bytes_mod_order_wide(&xi_bytes);

    // Get powers of the challenge and confirm they are nonzero
    let mut xi_powers = Vec::with_capacity(m as usize + 1);
    let mut xi_power = Scalar::ONE;
    for _ in 0..=m {
        if xi_power == Scalar::ZERO {
            return Err(ProofError::InvalidChallenge);
        }

        xi_powers.push(xi_power);
        xi_power *= xi;
    }

    Ok(xi_powers)
}

impl Proof {
    /// Generate a Triptych proof.
    ///
    /// The proof is generated by supplying a witness `witness` and corresponding statement `statement`.
    /// If the witness and statement do not share the same parameters, or if the statement is invalid for the witness,
    /// returns an error.
    ///
    /// You must also supply a cryptographically-secure random number generator `rng`.
    ///
    /// You may optionally provide a byte slice `message` that is bound to the proof's Fiat-Shamir transcript.
    /// The verifier must provide the same message in order for the proof to verify.
    #[allow(non_snake_case)]
    #[allow(clippy::too_many_lines)]
    pub fn prove<R: CryptoRngCore>(
        witness: &Witness,
        statement: &Statement,
        message: Option<&[u8]>,
        rng: &mut R,
    ) -> Result<Self, ProofError> {
        // Check that the witness and statement have identical parameters
        if witness.get_params() != statement.get_params() {
            return Err(ProofError::InvalidParameter);
        }

        // Extract values for convenience
        let r = witness.get_r();
        let l = witness.get_l();
        let M = statement.get_input_set().get_keys();
        let params = statement.get_params();
        let J = statement.get_J();

        // Check that the witness is valid against the statement
        if M.get(l as usize).ok_or(ProofError::InvalidParameter)? != &(r * params.get_G()) {
            return Err(ProofError::InvalidParameter);
        }
        if &(r * J) != params.get_U() {
            return Err(ProofError::InvalidParameter);
        }

        // Start the transcript
        let mut transcript = Transcript::new("Triptych proof".as_bytes());
        transcript.append_u64("version".as_bytes(), VERSION);
        if let Some(message) = message {
            transcript.append_message("message".as_bytes(), message);
        }
        transcript.append_message("params".as_bytes(), params.get_hash());
        transcript.append_message("M".as_bytes(), statement.get_input_set().get_hash());
        transcript.append_message("J".as_bytes(), J.compress().as_bytes());

        // Compute the `A` matrix commitment
        let r_A = Scalar::random(rng);
        let mut a = (0..params.get_m())
            .map(|_| {
                (0..params.get_n())
                    .map(|_| Scalar::random(rng))
                    .collect::<Vec<Scalar>>()
            })
            .collect::<Vec<Vec<Scalar>>>();
        for j in (0..params.get_m()).map(|j| j as usize) {
            a[j][0] = -a[j][1..].iter().sum::<Scalar>();
        }
        let A = params
            .commit_matrix(&a, &r_A)
            .map_err(|_| ProofError::InvalidParameter)?;

        // Compute the `B` matrix commitment
        let r_B = Scalar::random(rng);
        let l_decomposed = params.decompose(l).map_err(|_| ProofError::InvalidParameter)?;
        let sigma = (0..params.get_m())
            .map(|j| {
                (0..params.get_n())
                    .map(|i| delta(l_decomposed[j as usize], i))
                    .collect::<Vec<Scalar>>()
            })
            .collect::<Vec<Vec<Scalar>>>();
        let B = params
            .commit_matrix(&sigma, &r_B)
            .map_err(|_| ProofError::InvalidParameter)?;

        // Compute the `C` matrix commitment
        let two = Scalar::from(2u32);
        let r_C = Scalar::random(rng);
        let a_sigma = (0..params.get_m())
            .map(|j| {
                (0..params.get_n())
                    .map(|i| a[j as usize][i as usize] * (Scalar::ONE - two * sigma[j as usize][i as usize]))
                    .collect::<Vec<Scalar>>()
            })
            .collect::<Vec<Vec<Scalar>>>();
        let C = params
            .commit_matrix(&a_sigma, &r_C)
            .map_err(|_| ProofError::InvalidParameter)?;

        // Compute the `D` matrix commitment
        let r_D = Scalar::random(rng);
        let a_square = (0..params.get_m())
            .map(|j| {
                (0..params.get_n())
                    .map(|i| -a[j as usize][i as usize] * a[j as usize][i as usize])
                    .collect::<Vec<Scalar>>()
            })
            .collect::<Vec<Vec<Scalar>>>();
        let D = params
            .commit_matrix(&a_square, &r_D)
            .map_err(|_| ProofError::InvalidParameter)?;

        // Random masks
        let rho = Zeroizing::new(
            (0..params.get_m())
                .map(|_| Scalar::random(rng))
                .collect::<Vec<Scalar>>(),
        );

        // Compute `p` polynomial vector coefficients using repeated convolution
        let mut p = Vec::<Vec<Scalar>>::with_capacity(params.get_N() as usize);
        for k in 0..params.get_N() {
            let k_decomposed = params.decompose(k).map_err(|_| ProofError::InvalidParameter)?;

            // Set the initial coefficients using the first degree-one polynomial (`j = 0`)
            let mut coefficients = Vec::new();
            coefficients.resize(params.get_m() as usize + 1, Scalar::ZERO);
            coefficients[0] = a[0][k_decomposed[0] as usize];
            coefficients[1] = sigma[0][k_decomposed[0] as usize];

            // Use convolution against each remaining degree-one polynomial
            for j in 1..params.get_m() {
                // For the degree-zero portion, simply multiply each coefficient accordingly
                let degree_0_portion = coefficients
                    .iter()
                    .map(|c| a[j as usize][k_decomposed[j as usize] as usize] * c)
                    .collect::<Vec<Scalar>>();

                // For the degree-one portion, we also need to increase each exponent by one
                // Rotating the coefficients is fine here since the highest is always zero!
                let mut shifted_coefficients = coefficients.clone();
                shifted_coefficients.rotate_right(1);
                let degree_1_portion = shifted_coefficients
                    .iter()
                    .map(|c| sigma[j as usize][k_decomposed[j as usize] as usize] * c)
                    .collect::<Vec<Scalar>>();

                coefficients = degree_0_portion
                    .iter()
                    .zip(degree_1_portion.iter())
                    .map(|(x, y)| x + y)
                    .collect::<Vec<Scalar>>();
            }

            p.push(coefficients);
        }

        // Compute `X` vector
        let X = rho
            .iter()
            .enumerate()
            .map(|(j, rho)| {
                let X_points = M.iter().chain(once(params.get_G()));
                let X_scalars = p.iter().map(|p| &p[j]).chain(once(rho));

                RistrettoPoint::multiscalar_mul(X_scalars, X_points)
            })
            .collect::<Vec<RistrettoPoint>>();

        // Compute `Y` vector
        let Y = rho.iter().map(|rho| rho * J).collect::<Vec<RistrettoPoint>>();

        // Update the transcript
        transcript.append_message("A".as_bytes(), A.compress().as_bytes());
        transcript.append_message("B".as_bytes(), B.compress().as_bytes());
        transcript.append_message("C".as_bytes(), C.compress().as_bytes());
        transcript.append_message("D".as_bytes(), D.compress().as_bytes());
        for item in &X {
            transcript.append_message("X".as_bytes(), item.compress().as_bytes());
        }
        for item in &Y {
            transcript.append_message("Y".as_bytes(), item.compress().as_bytes());
        }

        // Get challenge powers
        let xi_powers = xi_powers(&mut transcript, params.get_m())?;

        // Compute the `f` matrix
        let f = (0..params.get_m())
            .map(|j| {
                (1..params.get_n())
                    .map(|i| sigma[j as usize][i as usize] * xi_powers[1] + a[j as usize][i as usize])
                    .collect::<Vec<Scalar>>()
            })
            .collect::<Vec<Vec<Scalar>>>();

        // Compute the remaining response values
        let z_A = r_A + xi_powers[1] * r_B;
        let z_C = xi_powers[1] * r_C + r_D;
        let z = r * xi_powers[params.get_m() as usize] -
            rho.iter()
                .zip(xi_powers.iter())
                .map(|(rho, xi_power)| rho * xi_power)
                .sum::<Scalar>();

        Ok(Self {
            A,
            B,
            C,
            D,
            X,
            Y,
            f,
            z_A,
            z_C,
            z,
        })
    }

    /// Verify a Triptych proof.
    ///
    /// Verification requires that the statement `statement` and optional byte slice `message` match those used when the
    /// proof was generated.
    ///
    /// You must also supply a cryptographically-secure random number generator `rng` that is used internally for
    /// efficiency.
    ///
    /// Returns a boolean that is `true` if and only if the proof is valid.
    #[allow(non_snake_case)]
    pub fn verify<R: CryptoRngCore>(&self, statement: &Statement, message: Option<&[u8]>, rng: &mut R) -> bool {
        // Extract statement values for convenience
        let M = statement.get_input_set().get_keys();
        let params = statement.get_params();
        let J = statement.get_J();

        // Generate the verifier challenge
        let mut transcript = Transcript::new("Triptych proof".as_bytes());
        transcript.append_u64("version".as_bytes(), VERSION);
        if let Some(message) = message {
            transcript.append_message("message".as_bytes(), message);
        }
        transcript.append_message("params".as_bytes(), params.get_hash());
        transcript.append_message("M".as_bytes(), statement.get_input_set().get_hash());
        transcript.append_message("J".as_bytes(), J.compress().as_bytes());

        transcript.append_message("A".as_bytes(), self.A.compress().as_bytes());
        transcript.append_message("B".as_bytes(), self.B.compress().as_bytes());
        transcript.append_message("C".as_bytes(), self.C.compress().as_bytes());
        transcript.append_message("D".as_bytes(), self.D.compress().as_bytes());
        for item in &self.X {
            transcript.append_message("X".as_bytes(), item.compress().as_bytes());
        }
        for item in &self.Y {
            transcript.append_message("Y".as_bytes(), item.compress().as_bytes());
        }

        // Get challenge powers
        let xi_powers = match xi_powers(&mut transcript, params.get_m()) {
            Ok(xi_powers) => xi_powers,
            _ => {
                return false;
            },
        };

        // Reconstruct the remaining `f` terms
        let f = (0..params.get_m())
            .map(|j| {
                let mut f_j = Vec::with_capacity(params.get_n() as usize);
                f_j.push(xi_powers[1] - self.f[j as usize].iter().sum::<Scalar>());
                f_j.extend(self.f[j as usize].iter());
                f_j
            })
            .collect::<Vec<Vec<Scalar>>>();

        // Generate weights for verification equations
        // We implicitly set `w3 = 1` to avoid unnecessary constant-time multiplication
        let w1 = Scalar::random(rng);
        let w2 = Scalar::random(rng);
        let w4 = Scalar::random(rng);

        // Set up the point iterator for the final check
        let points = once(params.get_G())
            .chain(params.get_CommitmentG().iter())
            .chain(once(params.get_CommitmentH()))
            .chain(once(&self.A))
            .chain(once(&self.B))
            .chain(once(&self.C))
            .chain(once(&self.D))
            .chain(once(J))
            .chain(self.X.iter())
            .chain(self.Y.iter())
            .chain(M.iter())
            .chain(once(params.get_U()));

        // Set up the scalar vector for the final check, matching the point iterator
        let mut scalars =
            Vec::with_capacity((params.get_N() + 2 * params.get_m() + params.get_n() * params.get_m() + 8) as usize);
        let mut U_scalar = Scalar::ZERO;

        // G
        scalars.push(-self.z);

        // CommitmentG
        for f_row in &f {
            for f_item in f_row {
                scalars.push(w1 * f_item + w2 * f_item * (xi_powers[1] - f_item));
            }
        }

        // CommitmentH
        scalars.push(w1 * self.z_A + w2 * self.z_C);

        // A
        scalars.push(-w1);

        // B
        scalars.push(-w1 * xi_powers[1]);

        // C
        scalars.push(-w2 * xi_powers[1]);

        // D
        scalars.push(-w2);

        // J
        scalars.push(-w4 * self.z);

        // X
        for xi_power in &xi_powers[0..(params.get_m() as usize)] {
            scalars.push(-xi_power);
        }

        // Y
        for xi_power in &xi_powers[0..(params.get_m() as usize)] {
            scalars.push(-w4 * xi_power);
        }

        // M
        for k in 0..params.get_N() {
            let k_decomposed = match params.decompose(k) {
                Ok(k_decomposed) => k_decomposed,
                _ => return false,
            };
            let f_product = (0..params.get_m())
                .map(|j| f[j as usize][k_decomposed[j as usize] as usize])
                .product::<Scalar>();

            scalars.push(f_product);
            U_scalar += f_product;
        }

        // U
        scalars.push(w4 * U_scalar);

        // Perform the final check; this can be done in variable time since it holds no secrets
        RistrettoPoint::vartime_multiscalar_mul(scalars.iter(), points) == RistrettoPoint::identity()
    }
}

#[cfg(test)]
mod test {
    use alloc::{sync::Arc, vec::Vec};

    use curve25519_dalek::RistrettoPoint;
    use rand_chacha::ChaCha12Rng;
    use rand_core::{CryptoRngCore, SeedableRng};

    use crate::{
        parameters::Parameters,
        proof::Proof,
        statement::{InputSet, Statement},
        witness::Witness,
    };

    // Generate a witness and corresponding statement
    #[allow(non_snake_case)]
    fn generate_data<R: CryptoRngCore>(n: u32, m: u32, rng: &mut R) -> (Witness, Statement) {
        // Generate parameters
        let params = Arc::new(Parameters::new(n, m).unwrap());

        // Generate witness
        let witness = Witness::random(&params, rng);

        // Generate input set
        let M = (0..params.get_N())
            .map(|i| {
                if i == witness.get_l() {
                    witness.compute_verification_key()
                } else {
                    RistrettoPoint::random(rng)
                }
            })
            .collect::<Vec<RistrettoPoint>>();
        let input_set = Arc::new(InputSet::new(&M));

        // Generate statement
        let J = witness.compute_linking_tag();
        let statement = Statement::new(&params, &input_set, &J).unwrap();

        (witness, statement)
    }

    #[test]
    #[allow(non_snake_case)]
    #[allow(non_upper_case_globals)]
    fn test_prove_verify() {
        // Generate data
        const n: u32 = 2;
        const m: u32 = 4;
        let mut rng = ChaCha12Rng::seed_from_u64(8675309);
        let (witness, statement) = generate_data(n, m, &mut rng);

        // Generate and verify a proof
        let message = "Proof messsage".as_bytes();
        let proof = Proof::prove(&witness, &statement, Some(message), &mut rng).unwrap();
        assert!(proof.verify(&statement, Some(message), &mut rng));
    }

    #[test]
    #[allow(non_snake_case)]
    #[allow(non_upper_case_globals)]
    fn test_evil_message() {
        // Generate data
        const n: u32 = 2;
        const m: u32 = 4;
        let mut rng = ChaCha12Rng::seed_from_u64(8675309);
        let (witness, statement) = generate_data(n, m, &mut rng);

        // Generate a proof
        let message = "Proof messsage".as_bytes();
        let proof = Proof::prove(&witness, &statement, Some(message), &mut rng).unwrap();

        // Attempt to verify the proof against a different message, which should fail
        let evil_message = "Evil proof message".as_bytes();
        assert!(!proof.verify(&statement, Some(evil_message), &mut rng));
    }

    #[test]
    #[allow(non_snake_case)]
    #[allow(non_upper_case_globals)]
    fn test_evil_input_set() {
        // Generate data
        const n: u32 = 2;
        const m: u32 = 4;
        let mut rng = ChaCha12Rng::seed_from_u64(8675309);
        let (witness, statement) = generate_data(n, m, &mut rng);

        // Generate a proof
        let message = "Proof messsage".as_bytes();
        let proof = Proof::prove(&witness, &statement, Some(message), &mut rng).unwrap();

        // Generate a statement with a modified input set
        let mut M = statement.get_input_set().get_keys().to_vec();
        let index = ((witness.get_l() + 1) % witness.get_params().get_N()) as usize;
        M[index] = RistrettoPoint::random(&mut rng);
        let evil_input_set = Arc::new(InputSet::new(&M));
        let evil_statement = Statement::new(statement.get_params(), &evil_input_set, statement.get_J()).unwrap();

        // Attempt to verify the proof against the new statement, which should fail
        assert!(!proof.verify(&evil_statement, Some(message), &mut rng));
    }

    #[test]
    #[allow(non_snake_case)]
    #[allow(non_upper_case_globals)]
    fn test_evil_linking_tag() {
        // Generate data
        const n: u32 = 2;
        const m: u32 = 4;
        let mut rng = ChaCha12Rng::seed_from_u64(8675309);
        let (witness, statement) = generate_data(n, m, &mut rng);

        // Generate a proof
        let message = "Proof messsage".as_bytes();
        let proof = Proof::prove(&witness, &statement, Some(message), &mut rng).unwrap();

        // Generate a statement with a modified linking tag
        let evil_statement = Statement::new(
            statement.get_params(),
            statement.get_input_set(),
            &RistrettoPoint::random(&mut rng),
        )
        .unwrap();

        // Attempt to verify the proof against the new statement, which should fail
        assert!(!proof.verify(&evil_statement, Some(message), &mut rng));
    }
}
