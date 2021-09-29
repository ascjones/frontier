use scale_info::TypeInfo;
use codec::{Encode, Decode};
use sp_runtime::{
	RuntimeDebug,
	traits::{
		self, Checkable, Extrinsic, ExtrinsicMetadata, IdentifyAccount, MaybeDisplay, Member,
		SignedExtension,
	},
	transaction_validity::{InvalidTransaction, TransactionValidityError},
};
use frame_support::traits::ExtrinsicCall;
use frame_support::weights::GetDispatchInfo;
use frame_support::weights::DispatchInfo;
use crate::{CheckedExtrinsic, CheckedSignature, SelfContainedCall};

/// A extrinsic right from the external world. This is unchecked and so
/// can contain a signature.
#[derive(PartialEq, Eq, Clone, Encode, Decode, RuntimeDebug, TypeInfo)]
pub struct UncheckedExtrinsic<Address, Call, Signature, Extra: SignedExtension>(
	pub sp_runtime::generic::UncheckedExtrinsic<Address, Call, Signature, Extra>,
);

#[cfg(feature = "std")]
impl<Address, Call, Signature, Extra> parity_util_mem::MallocSizeOf
	for UncheckedExtrinsic<Address, Call, Signature, Extra>
where
	Extra: SignedExtension,
{
	fn size_of(&self, ops: &mut parity_util_mem::MallocSizeOfOps) -> usize {
		self.0.size_of(ops)
	}
}

impl<Address, Call, Signature, Extra: SignedExtension>
	UncheckedExtrinsic<Address, Call, Signature, Extra>
{
	/// New instance of a signed extrinsic aka "transaction".
	pub fn new_signed(function: Call, signed: Address, signature: Signature, extra: Extra) -> Self {
		Self(sp_runtime::generic::UncheckedExtrinsic::new_signed(function, signed, signature, extra))
	}

	/// New instance of an unsigned extrinsic aka "inherent".
	pub fn new_unsigned(function: Call) -> Self {
		Self(sp_runtime::generic::UncheckedExtrinsic::new_unsigned(function))
	}
}

impl<Address, Call: SelfContainedCall, Signature, Extra: SignedExtension> Extrinsic
	for UncheckedExtrinsic<Address, Call, Signature, Extra>
{
	type Call = Call;

	type SignaturePayload = (Address, Signature, Extra);

	fn is_signed(&self) -> Option<bool> {
		if self.0.function.is_self_contained() {
			Some(true)
		} else {
			self.0.is_signed()
		}
	}

	fn new(function: Call, signed_data: Option<Self::SignaturePayload>) -> Option<Self> {
		sp_runtime::generic::UncheckedExtrinsic::new(function, signed_data).map(Self)
	}
}

impl<Address, AccountId, Call, Signature, Extra, Lookup> Checkable<Lookup>
	for UncheckedExtrinsic<Address, Call, Signature, Extra>
where
	Address: Member + MaybeDisplay,
	Call: Encode + Member + SelfContainedCall,
	Signature: Member + traits::Verify,
	<Signature as traits::Verify>::Signer: IdentifyAccount<AccountId = AccountId>,
	Extra: SignedExtension<AccountId = AccountId>,
	AccountId: Member + MaybeDisplay,
	Lookup: traits::Lookup<Source = Address, Target = AccountId>,
{
	type Checked = CheckedExtrinsic<AccountId, Call, Extra, <Call as SelfContainedCall>::SignedInfo>;

	fn check(self, lookup: &Lookup) -> Result<Self::Checked, TransactionValidityError> {
		if self.0.function.is_self_contained() {
			if self.0.signature.is_some() {
				return Err(TransactionValidityError::Invalid(InvalidTransaction::BadProof))
			}

			let signed_info = self.0.function.check_self_contained().ok_or(TransactionValidityError::Invalid(InvalidTransaction::BadProof))??;
			Ok(CheckedExtrinsic {
				signed: CheckedSignature::SelfContained(signed_info),
				function: self.0.function,
			})
		} else {
			let checked = Checkable::<Lookup>::check(self.0, lookup)?;
			Ok(CheckedExtrinsic {
				signed: match checked.signed {
					Some((id, extra)) => CheckedSignature::Signed(id, extra),
					None => CheckedSignature::Unsigned,
				},
				function: checked.function,
			})
		}
	}
}

impl<Address, Call, Signature, Extra> ExtrinsicMetadata
	for UncheckedExtrinsic<Address, Call, Signature, Extra>
where
	Extra: SignedExtension,
{
	const VERSION: u8 = <sp_runtime::generic::UncheckedExtrinsic<Address, Call, Signature, Extra> as ExtrinsicMetadata>::VERSION;
	type SignedExtensions = Extra;
}

impl<Address, Call: SelfContainedCall, Signature, Extra> ExtrinsicCall
	for UncheckedExtrinsic<Address, Call, Signature, Extra>
where
	Extra: SignedExtension,
{
	fn call(&self) -> &Self::Call {
		&self.0.function
	}
}

impl<Address, Call: GetDispatchInfo, Signature, Extra> GetDispatchInfo
	for UncheckedExtrinsic<Address, Call, Signature, Extra>
where
	Extra: SignedExtension,
{
	fn get_dispatch_info(&self) -> DispatchInfo {
		self.0.function.get_dispatch_info()
	}
}
