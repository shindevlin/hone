#![forbid(unsafe_code)]

use core::fmt;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AccountState {
    pub balance: u128,
    pub staked: u128,
    pub delegated: u128,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RewardSplit {
    pub mining: u128,
    pub verifier: u128,
    pub clock: u128,
    pub storage: u128,
    pub sensor: u128,
    pub service: u128,
    pub recycle: u128,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccountingError {
    InsufficientBalance { have: u128, need: u128 },
    InsufficientStake { have: u128, need: u128 },
    Overflow,
    InvalidGenesis,
}

impl fmt::Display for AccountingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AccountingError::InsufficientBalance { have, need } => {
                write!(f, "insufficient balance: have {}, need {}", have, need)
            }
            AccountingError::InsufficientStake { have, need } => {
                write!(f, "insufficient stake: have {}, need {}", have, need)
            }
            AccountingError::Overflow => write!(f, "accounting overflow"),
            AccountingError::InvalidGenesis => write!(f, "invalid non-zero genesis state"),
        }
    }
}

pub type Result<T> = core::result::Result<T, AccountingError>;

pub const DEFAULT_DECIMALS: u32 = 10;

pub fn assert_zero_genesis(accounts: &[AccountState]) -> Result<()> {
    if accounts.iter().any(|a| a.balance != 0 || a.staked != 0 || a.delegated != 0) {
        return Err(AccountingError::InvalidGenesis);
    }
    Ok(())
}

pub fn apply_transfer(from: &mut AccountState, to: &mut AccountState, amount: u128) -> Result<()> {
    if from.balance < amount {
        return Err(AccountingError::InsufficientBalance { have: from.balance, need: amount });
    }
    from.balance -= amount;
    to.balance = to.balance.checked_add(amount).ok_or(AccountingError::Overflow)?;
    Ok(())
}

pub fn apply_stake(account: &mut AccountState, amount: u128) -> Result<()> {
    if account.balance < amount {
        return Err(AccountingError::InsufficientBalance { have: account.balance, need: amount });
    }
    account.balance -= amount;
    account.staked = account.staked.checked_add(amount).ok_or(AccountingError::Overflow)?;
    Ok(())
}

pub fn apply_unstake(account: &mut AccountState, amount: u128) -> Result<()> {
    if account.staked < amount {
        return Err(AccountingError::InsufficientStake { have: account.staked, need: amount });
    }
    account.staked -= amount;
    account.balance = account.balance.checked_add(amount).ok_or(AccountingError::Overflow)?;
    Ok(())
}

pub fn apply_delegate(account: &mut AccountState, amount: u128) -> Result<()> {
    if account.balance < amount {
        return Err(AccountingError::InsufficientBalance { have: account.balance, need: amount });
    }
    account.balance -= amount;
    account.delegated = account.delegated.checked_add(amount).ok_or(AccountingError::Overflow)?;
    Ok(())
}

pub fn apply_undelegate(account: &mut AccountState, amount: u128) -> Result<()> {
    if account.delegated < amount {
        return Err(AccountingError::InsufficientBalance { have: account.delegated, need: amount });
    }
    account.delegated -= amount;
    account.balance = account.balance.checked_add(amount).ok_or(AccountingError::Overflow)?;
    Ok(())
}

pub fn add_to_recycle(recycle: &mut AccountState, amount: u128) -> Result<()> {
    recycle.balance = recycle.balance.checked_add(amount).ok_or(AccountingError::Overflow)?;
    Ok(())
}

pub fn split_reward(total: u128) -> RewardSplit {
    let mining = total * 55 / 100;
    let verifier = total * 10 / 100;
    let clock = total * 5 / 100;
    let storage = total * 15 / 100;
    let sensor = total * 7 / 100;
    let service = total * 6 / 100;
    let allocated = mining + verifier + clock + storage + sensor + service;
    let recycle = total.saturating_sub(allocated);

    RewardSplit {
        mining,
        verifier,
        clock,
        storage,
        sensor,
        service,
        recycle,
    }
}

pub fn assert_non_negative(account: &AccountState) -> Result<()> {
    let _ = account;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_genesis_passes() {
        assert!(assert_zero_genesis(&[AccountState::default(), AccountState::default()]).is_ok());
    }

    #[test]
    fn non_zero_genesis_fails() {
        let mut a = AccountState::default();
        a.balance = 1;
        assert!(assert_zero_genesis(&[a]).is_err());
    }

    #[test]
    fn transfer_rejects_overspend() {
        let mut from = AccountState { balance: 5, staked: 0, delegated: 0 };
        let mut to = AccountState::default();
        assert!(apply_transfer(&mut from, &mut to, 6).is_err());
        assert_eq!(from.balance, 5);
        assert_eq!(to.balance, 0);
    }

    #[test]
    fn recycle_split_conserves_total() {
        let total = 1_000u128;
        let split = split_reward(total);
        assert_eq!(split.mining + split.verifier + split.clock + split.storage + split.sensor + split.service + split.recycle, total);
    }
}

