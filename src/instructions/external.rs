use crate::{common::address_to_u256, host::*, state::ExecutionState};
use ethereum_types::U256;

pub(crate) fn address(state: &mut ExecutionState) {
    state.stack.push(address_to_u256(state.message.destination));
}

pub(crate) fn caller(state: &mut ExecutionState) {
    state.stack.push(address_to_u256(state.message.sender));
}

pub(crate) fn callvalue(state: &mut ExecutionState) {
    state.stack.push(state.message.value);
}

#[doc(hidden)]
#[macro_export]
macro_rules! balance {
    ($co:expr, $state:expr) => {
        use crate::{
            common::*,
            continuation::{interrupt_data::*, resume_data::*},
            host::*,
            instructions::properties::*,
        };

        let address = u256_to_address($state.stack.pop());

        if $state.evm_revision >= Revision::Berlin {
            let access_status = ResumeDataVariant::into_access_account_status(
                $co.yield_(InterruptDataVariant::AccessAccount(AccessAccount {
                    address,
                }))
                .await,
            )
            .unwrap()
            .status;
            if access_status == AccessStatus::Cold {
                $state.gas_left -= i64::from(ADDITIONAL_COLD_ACCOUNT_ACCESS_COST);
                if $state.gas_left < 0 {
                    return Err(StatusCode::OutOfGas);
                }
            }
        }

        let balance = ResumeDataVariant::into_balance(
            $co.yield_(InterruptDataVariant::GetBalance(GetBalance { address }))
                .await,
        )
        .unwrap()
        .balance;

        $state.stack.push(balance);
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! extcodesize {
    ($co:expr, $state:expr) => {
        use crate::{
            common::*,
            continuation::{interrupt_data::*, resume_data::*},
            host::*,
            instructions::properties::*,
        };

        let address = u256_to_address($state.stack.pop());

        if $state.evm_revision >= Revision::Berlin {
            let access_account = ResumeDataVariant::into_access_account_status(
                $co.yield_(InterruptDataVariant::AccessAccount(AccessAccount {
                    address,
                }))
                .await,
            )
            .unwrap()
            .status;
            if access_account == AccessStatus::Cold {
                $state.gas_left -= i64::from(ADDITIONAL_COLD_ACCOUNT_ACCESS_COST);
                if $state.gas_left < 0 {
                    return Err(StatusCode::OutOfGas);
                }
            }
        }

        let code_size = ResumeDataVariant::into_code_size(
            $co.yield_(InterruptDataVariant::GetCodeSize(GetCodeSize { address }))
                .await,
        )
        .unwrap()
        .code_size;
        $state.stack.push(code_size);
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! push_txcontext {
    ($co:expr, $state:expr, $accessor:expr) => {
        use $crate::continuation::{interrupt_data::*, resume_data::*};

        let tx_context = ResumeDataVariant::into_tx_context_data(
            $co.yield_(InterruptDataVariant::GetTxContext).await,
        )
        .unwrap()
        .context;

        $state.stack.push($accessor(tx_context));
    };
}

pub(crate) fn origin_accessor(tx_context: TxContext) -> U256 {
    address_to_u256(tx_context.tx_origin)
}

pub(crate) fn coinbase_accessor(tx_context: TxContext) -> U256 {
    address_to_u256(tx_context.block_coinbase)
}

pub(crate) fn gasprice_accessor(tx_context: TxContext) -> U256 {
    tx_context.tx_gas_price
}

pub(crate) fn timestamp_accessor(tx_context: TxContext) -> U256 {
    tx_context.block_timestamp.into()
}

pub(crate) fn number_accessor(tx_context: TxContext) -> U256 {
    tx_context.block_number.into()
}

pub(crate) fn gaslimit_accessor(tx_context: TxContext) -> U256 {
    tx_context.block_gas_limit.into()
}

pub(crate) fn difficulty_accessor(tx_context: TxContext) -> U256 {
    tx_context.block_difficulty
}

pub(crate) fn chainid_accessor(tx_context: TxContext) -> U256 {
    tx_context.chain_id
}

pub(crate) fn basefee_accessor(tx_context: TxContext) -> U256 {
    tx_context.block_base_fee
}

#[doc(hidden)]
#[macro_export]
macro_rules! selfbalance {
    ($co:expr, $state:expr) => {{
        use $crate::continuation::{interrupt_data::*, resume_data::*};

        let balance = ResumeDataVariant::into_balance(
            $co.yield_(InterruptDataVariant::GetBalance(GetBalance {
                address: $state.message.destination,
            }))
            .await,
        )
        .unwrap()
        .balance;

        $state.stack.push(balance);
    }};
}

#[doc(hidden)]
#[macro_export]
macro_rules! blockhash {
    ($co:expr, $state:expr) => {
        use ethereum_types::H256;
        use $crate::continuation::{interrupt_data::*, resume_data::*};

        let number = $state.stack.pop();

        let upper_bound = ResumeDataVariant::into_tx_context_data(
            $co.yield_(InterruptDataVariant::GetTxContext).await,
        )
        .unwrap()
        .context
        .block_number;
        let lower_bound = upper_bound.saturating_sub(256);

        let mut header = H256::zero();
        if number <= u64::MAX.into() {
            let n = number.as_u64();
            if (lower_bound..upper_bound).contains(&n) {
                header = ResumeDataVariant::into_block_hash(
                    $co.yield_(InterruptDataVariant::GetBlockHash(GetBlockHash {
                        block_number: n,
                    }))
                    .await,
                )
                .unwrap()
                .hash;
            }
        }

        $state.stack.push(U256::from_big_endian(&header.0));
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! do_log {
    ($co:expr, $state:expr, $num_topics:expr) => {{
        use arrayvec::ArrayVec;
        use ethereum_types::H256;
        use $crate::continuation::{interrupt_data::*, resume_data::*};

        if $state.message.is_static {
            return Err(StatusCode::StaticModeViolation);
        }

        let offset = $state.stack.pop();
        let size = $state.stack.pop();

        let region =
            memory::verify_memory_region($state, offset, size).map_err(|_| StatusCode::OutOfGas)?;

        if let Some(region) = &region {
            let cost = region.size.get() as i64 * 8;
            $state.gas_left -= cost;
            if cost < 0 {
                return Err(StatusCode::OutOfGas);
            }
        }

        let mut topics = ArrayVec::new();
        for _ in 0..$num_topics {
            topics.push(H256($state.stack.pop().into()));
        }

        let data = if let Some(region) = region {
            &$state.memory[region.offset..region.offset + region.size.get()]
        } else {
            &[]
        };
        let r = $co
            .yield_(InterruptDataVariant::EmitLog(EmitLog {
                address: $state.message.destination,
                data: data.to_vec().into(),
                topics,
            }))
            .await;

        assert!(matches!(r, ResumeDataVariant::Empty));
    }};
}

#[doc(hidden)]
#[macro_export]
macro_rules! sload {
    ($co:expr, $state:expr) => {{
        use ethereum_types::H256;
        use $crate::{
            continuation::{interrupt_data::*, resume_data::*},
            host::*,
            instructions::properties::{COLD_SLOAD_COST, WARM_STORAGE_READ_COST},
        };

        let key = H256($state.stack.pop().into());

        if $state.evm_revision >= Revision::Berlin {
            let access_status = ResumeDataVariant::into_access_storage_status(
                $co.yield_(InterruptDataVariant::AccessStorage(AccessStorage {
                    address: $state.message.destination,
                    key,
                }))
                .await,
            )
            .unwrap()
            .status;
            if access_status == AccessStatus::Cold {
                // The warm storage access cost is already applied (from the cost table).
                // Here we need to apply additional cold storage access cost.
                const ADDITIONAL_COLD_SLOAD_COST: u16 = COLD_SLOAD_COST - WARM_STORAGE_READ_COST;
                $state.gas_left -= i64::from(ADDITIONAL_COLD_SLOAD_COST);
                if $state.gas_left < 0 {
                    return Err(StatusCode::OutOfGas);
                }
            }
        }

        let storage = ResumeDataVariant::into_storage_value(
            $co.yield_(InterruptDataVariant::GetStorage(GetStorage {
                address: $state.message.destination,
                key,
            }))
            .await,
        )
        .unwrap()
        .value;

        $state.stack.push(U256::from_big_endian(storage.as_bytes()));
    }};
}

#[doc(hidden)]
#[macro_export]
macro_rules! sstore {
    ($co:expr, $state:expr) => {{
        use ethereum_types::H256;
        use $crate::{
            continuation::{interrupt_data::*, resume_data::*},
            host::*,
            instructions::properties::{COLD_SLOAD_COST, WARM_STORAGE_READ_COST},
        };

        if $state.message.is_static {
            return Err(StatusCode::StaticModeViolation);
        }

        if $state.evm_revision >= Revision::Istanbul && $state.gas_left <= 2300 {
            return Err(StatusCode::OutOfGas);
        }

        let key = H256($state.stack.pop().into());
        let value = H256($state.stack.pop().into());

        let mut cost = 0;
        if $state.evm_revision >= Revision::Berlin {
            let access_status = ResumeDataVariant::into_access_storage_status(
                $co.yield_(InterruptDataVariant::AccessStorage(AccessStorage {
                    address: $state.message.destination,
                    key,
                }))
                .await,
            )
            .unwrap()
            .status;

            if access_status == AccessStatus::Cold {
                cost = COLD_SLOAD_COST;
            }
        }

        let status = ResumeDataVariant::into_storage_status_info(
            $co.yield_(InterruptDataVariant::SetStorage(SetStorage {
                address: $state.message.destination,
                key,
                value,
            }))
            .await,
        )
        .unwrap()
        .status;

        cost = match status {
            StorageStatus::Unchanged | StorageStatus::ModifiedAgain => {
                if $state.evm_revision >= Revision::Berlin {
                    cost + WARM_STORAGE_READ_COST
                } else if $state.evm_revision == Revision::Istanbul {
                    800
                } else if $state.evm_revision == Revision::Constantinople {
                    200
                } else {
                    5000
                }
            }
            StorageStatus::Modified | StorageStatus::Deleted => {
                if $state.evm_revision >= Revision::Berlin {
                    cost + 5000 - COLD_SLOAD_COST
                } else {
                    5000
                }
            }
            StorageStatus::Added => cost + 20000,
        };
        $state.gas_left -= i64::from(cost);
        if $state.gas_left < 0 {
            return Err(StatusCode::OutOfGas);
        }
    }};
}

#[doc(hidden)]
#[macro_export]
macro_rules! selfdestruct {
    ($co:expr, $state:expr) => {{
        use crate::{
            common::*,
            continuation::{interrupt_data::*, resume_data::*},
            host::*,
            instructions::properties::*,
        };

        if $state.message.is_static {
            return Err(StatusCode::StaticModeViolation);
        }

        let beneficiary = u256_to_address($state.stack.pop());

        if $state.evm_revision >= Revision::Berlin {
            let access_status = ResumeDataVariant::into_access_account_status(
                $co.yield_(InterruptDataVariant::AccessAccount(AccessAccount {
                    address: beneficiary,
                }))
                .await,
            )
            .unwrap()
            .status;
            if access_status == AccessStatus::Cold {
                $state.gas_left -= i64::from(COLD_ACCOUNT_ACCESS_COST);
                if $state.gas_left < 0 {
                    return Err(StatusCode::OutOfGas);
                }
            }
        }

        if $state.evm_revision >= Revision::Tangerine
            && ($state.evm_revision == Revision::Tangerine
                || !{
                    ResumeDataVariant::into_balance(
                        $co.yield_(InterruptDataVariant::GetBalance(GetBalance {
                            address: $state.message.destination,
                        }))
                        .await,
                    )
                    .unwrap()
                    .balance
                    .is_zero()
                })
        {
            // After TANGERINE_WHISTLE apply additional cost of
            // sending value to a non-existing account.
            if !ResumeDataVariant::into_account_exists_status(
                $co.yield_(InterruptDataVariant::AccountExists(AccountExists {
                    address: beneficiary,
                }))
                .await,
            )
            .unwrap()
            .exists
            {
                $state.gas_left -= 25000;
                if $state.gas_left < 0 {
                    return Err(StatusCode::OutOfGas);
                }
            }
        }

        assert!(matches!(
            $co.yield_(InterruptDataVariant::Selfdestruct(Selfdestruct {
                address: $state.message.destination,
                beneficiary,
            }))
            .await,
            ResumeDataVariant::Empty
        ));
    }};
}

#[cfg(test)]
mod tests {
    use crate::common::u256_to_address;
    use ethereum_types::Address;
    use hex_literal::hex;

    #[test]
    fn u256_to_address_conversion() {
        assert_eq!(
            u256_to_address(0x42.into()),
            Address::from(hex!("0000000000000000000000000000000000000042"))
        );
    }
}
