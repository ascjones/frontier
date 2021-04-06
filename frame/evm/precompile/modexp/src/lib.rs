// SPDX-License-Identifier: Apache-2.0
// This file is part of Frontier.
//
// Copyright (c) 2020 Parity Technologies (UK) Ltd.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// 	http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

use alloc::vec::Vec;
use fp_evm::Precompile;
use evm::{ExitSucceed, ExitError, Context};
use num::{BigUint, Zero, One, ToPrimitive, FromPrimitive};

use core::ops::BitAnd;

pub struct Modexp;

const MIN_GAS_COST: u64 = 200;

// TODO: is there something in Substrate I can use?
fn max(a: u64, b: u64) -> u64 {
	if a > b {
		a
	} else {
		b
	}
}

// Calculate gas cost according to EIP 2565:
// https://eips.ethereum.org/EIPS/eip-2565
fn calculate_gas_cost(
	base_length: u64,
	exp_length: u64,
	mod_length: u64,
	exponent: &BigUint,
) -> u64 {

	fn calculate_multiplication_complexity(base_length: u64, mod_length: u64) -> u64 {
		let max_length = max(base_length, mod_length);
		let mut words = max_length / 8;
		if max_length % 8 > 0 {
			words += 1;
		}

		// TODO: prevent/handle overflow
		words * words
	}

	fn calculate_iteration_count(exp_length: u64, exponent: &BigUint) -> u64 {
		let mut iteration_count: u64 = 0;

		if exp_length <= 32 && exponent.is_zero() {
			iteration_count = 0;
		} else if exp_length <= 32 {
			iteration_count = exponent.bits() - 1;
		} else if exp_length > 32 {

			// construct BigUint to represent (2^256) - 1
			let bytes: [u8; 32] = [0xFF; 32];
			let max_256_bit_uint = BigUint::from_bytes_be(&bytes);

			iteration_count = (8 * (exp_length - 32)) + ((exponent.bitand(max_256_bit_uint)).bits() - 1);
		}

		max(iteration_count, 1)
	}

	let multiplication_complexity = calculate_multiplication_complexity(base_length, mod_length);
	let iteration_count = calculate_iteration_count(exp_length, exponent);
	let gas = max(MIN_GAS_COST,
				  multiplication_complexity * iteration_count / 3);

	gas
}

// ModExp expects the following as inputs:
// 1) 32 bytes expressing the length of base
// 2) 32 bytes expressing the length of exponent
// 3) 32 bytes expressing the length of modulus
// 4) base, size as described above
// 5) exponent, size as described above
// 6) modulus, size as described above
//
//
// NOTE: input sizes are arbitrarily large (up to 256 bits), with the expectation
//       that gas limits would be applied before actual computation.
//
//       maximum stack size will also prevent abuse.
//
//       see: https://eips.ethereum.org/EIPS/eip-198

impl Precompile for Modexp {
	fn execute(
		input: &[u8],
		target_gas: Option<u64>,
		_context: &Context,
	) -> core::result::Result<(ExitSucceed, Vec<u8>, u64), ExitError> {
		if input.len() < 96 {
			return Err(ExitError::Other("input must contain at least 96 bytes".into()));
		};

		// reasonable assumption: this must fit within the Ethereum EVM's max stack size
		let max_size_big = BigUint::from_u32(1024).expect("can't create BigUint");

		let mut buf = [0; 32];
		buf.copy_from_slice(&input[0..32]);
		let base_len_big = BigUint::from_bytes_be(&buf);
		if base_len_big > max_size_big {
			return Err(ExitError::Other("unreasonably large base length".into()));
		}

		buf.copy_from_slice(&input[32..64]);
		let exp_len_big = BigUint::from_bytes_be(&buf);
		if exp_len_big > max_size_big {
			return Err(ExitError::Other("unreasonably large exponent length".into()));
		}

		buf.copy_from_slice(&input[64..96]);
		let mod_len_big = BigUint::from_bytes_be(&buf);
		if mod_len_big > max_size_big {
			return Err(ExitError::Other("unreasonably large exponent length".into()));
		}

		// bounds check handled above
		let base_len = base_len_big.to_usize().expect("base_len out of bounds");
		let exp_len = exp_len_big.to_usize().expect("exp_len out of bounds");
		let mod_len = mod_len_big.to_usize().expect("mod_len out of bounds");

		// input length should be at least 96 + user-specified length of base + exp + mod
		let total_len = base_len + exp_len + mod_len + 96;
		if input.len() < total_len {
			return Err(ExitError::Other("insufficient input size".into()));
		}

		// Gas formula allows arbitrary large exp_len when base and modulus are empty, so we need to handle empty base first.
		let (r, gas_cost) = if base_len == 0 && mod_len == 0 {
			(BigUint::zero(), MIN_GAS_COST)
		} else {

			// read the numbers themselves.
			let base_start = 96; // previous 3 32-byte fields
			let base = BigUint::from_bytes_be(&input[base_start..base_start + base_len]);

			let exp_start = base_start + base_len;
			let exponent = BigUint::from_bytes_be(&input[exp_start..exp_start + exp_len]);

			// do our gas accounting
			// TODO: we could technically avoid reading base first...
			let gas_cost = calculate_gas_cost(base_len as u64, exp_len as u64, mod_len as u64, &exponent);
			if gas_cost > target_gas.expect("Gas limit not provided") { // TODO: I'm probably misunderstanding this
				return Err(ExitError::OutOfGas);
			}

			let mod_start = exp_start + exp_len;
			let modulus = BigUint::from_bytes_be(&input[mod_start..mod_start + mod_len]);

			if modulus.is_zero() || modulus.is_one() {
				(BigUint::zero(), gas_cost)
			} else {
				(base.modpow(&exponent, &modulus), gas_cost)
			}
		};

		// write output to given memory, left padded and same length as the modulus.
		let bytes = r.to_bytes_be();

		// always true except in the case of zero-length modulus, which leads to
		// output of length and value 1.
		if bytes.len() == mod_len {
			Ok((ExitSucceed::Returned, bytes.to_vec(), gas_cost))
		} else if bytes.len() < mod_len {
			let mut ret = Vec::with_capacity(mod_len);
			ret.extend(core::iter::repeat(0).take(mod_len - bytes.len()));
			ret.extend_from_slice(&bytes[..]);
			Ok((ExitSucceed::Returned, ret.to_vec(), gas_cost))
		} else {
			Err(ExitError::Other("failed".into()))
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	extern crate hex;
	use std::fs;
	use serde::Deserialize;

	#[allow(non_snake_case)]
	#[derive(Deserialize, Debug)]
	struct EthConsensusTest {
		Input: String,
		Expected: String,
		Name: String,
		Gas: u64,
	}

	#[test]
	fn process_consensus_tests() -> std::result::Result<(), ExitError> {
		let data = fs::read_to_string("./modexp_eip2565.json")
			.expect("Failed to read modexp_eip2565.json");

		let tests: Vec<EthConsensusTest> = serde_json::from_str(&data)
			.expect("expected json array");

		for test in tests {
			let input: Vec<u8> = hex::decode(test.Input)
				.expect("Could not hex-decode test input data");

			let cost: u64 = 1000000;

			let context: Context = Context {
				address: Default::default(),
				caller: Default::default(),
				apparent_value: From::from(0),
			};

			match Modexp::execute(&input, Some(cost), &context) {
				Ok((_, output, gas)) => {
					let as_hex: String = hex::encode(output);
					assert_eq!(as_hex, test.Expected,
							   "test '{}' failed (different output)", test.Name);
					assert_eq!(gas, test.Gas,
							   "test '{}' failed (different gas cost)", test.Name);
				},
				Err(_) => {
					panic!("Modexp::execute() returned error");
				}
			}

		}
		Ok(())
	}





	#[test]
	fn test_empty_input() -> std::result::Result<(), ExitError> {
		let input: [u8; 0] = [];

		let cost: u64 = 1;

		let context: Context = Context {
			address: Default::default(),
			caller: Default::default(),
			apparent_value: From::from(0),
		};

		match Modexp::execute(&input, Some(cost), &context) {
			Ok((_, _, _)) => {
				panic!("Test not expected to pass");
			},
			Err(e) => {
				assert_eq!(e, ExitError::Other("input must contain at least 96 bytes".into()));
				Ok(())
			}
		}
	}

	#[test]
	fn test_insufficient_input() -> std::result::Result<(), ExitError> {
		let input = hex::decode(
			"0000000000000000000000000000000000000000000000000000000000000001\
			0000000000000000000000000000000000000000000000000000000000000001\
			0000000000000000000000000000000000000000000000000000000000000001")
			.expect("Decode failed");

		let cost: u64 = 1;

		let context: Context = Context {
			address: Default::default(),
			caller: Default::default(),
			apparent_value: From::from(0),
		};

		match Modexp::execute(&input, Some(cost), &context) {
			Ok((_, _, _)) => {
				panic!("Test not expected to pass");
			},
			Err(e) => {
				assert_eq!(e, ExitError::Other("insufficient input size".into()));
				Ok(())
			}
		}
	}

	#[test]
	fn test_excessive_input() -> std::result::Result<(), ExitError> {

		let input = hex::decode(
			"1000000000000000000000000000000000000000000000000000000000000001\
			0000000000000000000000000000000000000000000000000000000000000001\
			0000000000000000000000000000000000000000000000000000000000000001")
			.expect("Decode failed");

		let cost: u64 = 1;

		let context: Context = Context {
			address: Default::default(),
			caller: Default::default(),
			apparent_value: From::from(0),
		};

		match Modexp::execute(&input, Some(cost), &context) {
			Ok((_, _, _)) => {
				panic!("Test not expected to pass");
			},
			Err(e) => {
				assert_eq!(e, ExitError::Other("unreasonably large base length".into()));
				Ok(())
			}
		}
	}

	#[test]
	fn test_simple_inputs() {

		let input = hex::decode(
			"0000000000000000000000000000000000000000000000000000000000000001\
			0000000000000000000000000000000000000000000000000000000000000001\
			0000000000000000000000000000000000000000000000000000000000000001\
			03\
			05\
			07").expect("Decode failed");

		// 3 ^ 5 % 7 == 5

		let cost: u64 = 100000;

		let context: Context = Context {
			address: Default::default(),
			caller: Default::default(),
			apparent_value: From::from(0),
		};

		match Modexp::execute(&input, Some(cost), &context) {
			Ok((_, output, _)) => {
				assert_eq!(output.len(), 1); // should be same length as mod
				let result = BigUint::from_bytes_be(&output[..]);
				let expected = BigUint::parse_bytes(b"5", 10).unwrap();
				assert_eq!(result, expected);
			},
			Err(_) => {
				panic!("Modexp::execute() returned error"); // TODO: how to pass error on?
			}
		}
	}

	#[test]
	fn test_large_inputs() {

		let input = hex::decode(
			"0000000000000000000000000000000000000000000000000000000000000020\
			0000000000000000000000000000000000000000000000000000000000000020\
			0000000000000000000000000000000000000000000000000000000000000020\
			000000000000000000000000000000000000000000000000000000000000EA5F\
			0000000000000000000000000000000000000000000000000000000000000015\
			0000000000000000000000000000000000000000000000000000000000003874")
			.expect("Decode failed");

		// 59999 ^ 21 % 14452 = 10055

		let cost: u64 = 100000;

		let context: Context = Context {
			address: Default::default(),
			caller: Default::default(),
			apparent_value: From::from(0),
		};

		match Modexp::execute(&input, Some(cost), &context) {
			Ok((_, output, _)) => {
				assert_eq!(output.len(), 32); // should be same length as mod
				let result = BigUint::from_bytes_be(&output[..]);
				let expected = BigUint::parse_bytes(b"10055", 10).unwrap();
				assert_eq!(result, expected);
			},
			Err(_) => {
				panic!("Modexp::execute() returned error"); // TODO: how to pass error on?
			}
		}
	}

	#[test]
	fn test_large_computation() {
		let input = hex::decode(
			"0000000000000000000000000000000000000000000000000000000000000001\
			0000000000000000000000000000000000000000000000000000000000000020\
			0000000000000000000000000000000000000000000000000000000000000020\
			03\
			fffffffffffffffffffffffffffffffffffffffffffffffffffffffefffffc2e\
			fffffffffffffffffffffffffffffffffffffffffffffffffffffffefffffc2f")
			.expect("Decode failed");

		let cost: u64 = 100000;

		let context: Context = Context {
			address: Default::default(),
			caller: Default::default(),
			apparent_value: From::from(0),
		};

		match Modexp::execute(&input, Some(cost), &context) {
			Ok((_, output, _)) => {
				assert_eq!(output.len(), 32); // should be same length as mod
				let result = BigUint::from_bytes_be(&output[..]);
				let expected = BigUint::parse_bytes(b"1", 10).unwrap();
				assert_eq!(result, expected);
			},
			Err(_) => {
				panic!("Modexp::execute() returned error"); // TODO: how to pass error on?
			}
		}
	}
}
