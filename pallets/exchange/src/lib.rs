#![cfg_attr(not(feature = "std"), no_std)]

/// Edit this file to define custom logic or remove it if it is not needed.
/// Learn more about FRAME and the core library of Substrate FRAME pallets:
/// https://substrate.dev/docs/en/knowledgebase/runtime/frame
pub use pallet::*;

#[cfg(test)]
mod mock;

#[cfg(test)]
mod tests;

#[frame_support::pallet]
pub mod pallet {
    use codec::{Decode, Encode};
    use frame_support::{
        dispatch::DispatchResultWithPostInfo, pallet_prelude::*, traits::BalanceStatus,
    };
    use frame_system::pallet_prelude::*;
    use orml_traits::{MultiCurrency, MultiReservableCurrency};
    use orml_utilities::with_transaction_result;
    use sp_runtime::{
        traits::{AtLeast32BitUnsigned, Bounded, CheckedAdd, MaybeSerializeDeserialize, One, Zero},
        DispatchResult, RuntimeDebug,
    };

    /// Configure the pallet by specifying the parameters and types on which it depends.
    #[pallet::config]
    pub trait Config: frame_system::Config {
        /// Because this pallet emits events, it depends on the runtime's definition of an event.
        type Event: From<Event<Self>> + IsType<<Self as frame_system::Config>::Event>;
        type Currency: MultiReservableCurrency<Self::AccountId>; // reserve 锁定货币，在进行交易之前
        type OrderId: Parameter
            + AtLeast32BitUnsigned
            + Default
            + Copy
            + MaybeSerializeDeserialize
            + Bounded;
    }

    #[derive(Encode, Decode, Clone, RuntimeDebug, PartialEq, Eq)]
    pub struct Order<CurrencyId, Balance, AccountId> {
        pub base_currency_id: CurrencyId,
        #[codec(compact)]
        pub base_amount: Balance,
        pub target_currency_id: CurrencyId,
        #[codec(compact)]
        pub target_amount: Balance,
        pub owner: AccountId,
    }

    type BalanceOf<T> =
        <<T as Config>::Currency as MultiCurrency<<T as frame_system::Config>::AccountId>>::Balance;
    type CurrencyIdOf<T> = <<T as Config>::Currency as MultiCurrency<
        <T as frame_system::Config>::AccountId,
    >>::CurrencyId;
    type OrderOf<T> = Order<CurrencyIdOf<T>, BalanceOf<T>, <T as frame_system::Config>::AccountId>;

    #[pallet::pallet]
    #[pallet::generate_store(pub(super) trait Store)]
    pub struct Pallet<T>(PhantomData<T>);

    #[pallet::storage]
    #[pallet::getter(fn orders)]
    pub type Orders<T: Config> =
        StorageMap<_, Twox64Concat, T::OrderId, Option<OrderOf<T>>, ValueQuery>;

    #[pallet::storage]
    #[pallet::getter(fn next_order_id)]
    pub type NextOrderId<T: Config> = StorageValue<_, T::OrderId, ValueQuery>;

    #[pallet::event]
    #[pallet::metadata(T::AccountId = "AccountId")]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        /// Event documentation should end with an array that provides descriptive names for event
        /// parameters. [something, who]
        OrderCreated(T::OrderId, OrderOf<T>),

        /// Event documentation should end with an array that provides descriptive names for event
        /// parameters. [something, who]
        OrderTaken(T::AccountId, T::OrderId, OrderOf<T>),

        /// Event documentation should end with an array that provides descriptive names for event
        /// parameters. [something, who]
        OrderCancelled(T::OrderId),
    }

    // Errors inform users that something went wrong.
    #[pallet::error]
    pub enum Error<T> {
        /// none value
        NoneValue,
        /// Order id overflow
        OrderIdOverflow,
        /// invalid order
        InvalidOrderId,
        /// in sufficient balance
        InsufficientBalance,
        /// not owner
        NotOwner,
        /// order is none
        OrderIsNone,
    }

    #[pallet::hooks]
    impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {}

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        #[pallet::weight(10_000 + T::DbWeight::get().writes(1))]
        pub fn submit_order(
            origin: OriginFor<T>,
            base_currency_id: CurrencyIdOf<T>,
            base_amount: BalanceOf<T>,
            target_currency_id: CurrencyIdOf<T>,
            target_amount: BalanceOf<T>,
        ) -> DispatchResultWithPostInfo {
            let who = ensure_signed(origin)?;

            NextOrderId::<T>::try_mutate(|id| -> DispatchResult {
                let order_id = *id;

                let order = Order {
                    base_currency_id,
                    base_amount,
                    target_currency_id,
                    target_amount,
                    owner: who.clone(),
                };

                *id = id
                    .checked_add(&One::one())
                    .ok_or(Error::<T>::OrderIdOverflow)?;

                T::Currency::reserve(base_currency_id, &who, base_amount)?;

                Orders::<T>::insert(order_id, &Some(order.clone()));

                Self::deposit_event(Event::OrderCreated(order_id, order));

                Ok(())
            })?;
            Ok(().into())
        }

        #[pallet::weight(10_000 + T::DbWeight::get().writes(1))]
        fn take_order(origin: OriginFor<T>, order_id: T::OrderId) -> DispatchResultWithPostInfo {
            let who = ensure_signed(origin)?;

            Orders::<T>::try_mutate_exists(order_id, |order| -> DispatchResult {
                let order = order
                    .take()
                    .ok_or(Error::<T>::InvalidOrderId)?
                    .ok_or(Error::<T>::OrderIsNone)?;

                with_transaction_result(|| {
                    T::Currency::transfer(
                        order.target_currency_id,
                        &who,
                        &order.owner,
                        order.target_amount,
                    )?;
                    let val = T::Currency::repatriate_reserved(
                        order.base_currency_id,
                        &order.owner,
                        &who,
                        order.base_amount,
                        BalanceStatus::Free,
                    )?;

                    ensure!(val.is_zero(), Error::<T>::InsufficientBalance);

                    Self::deposit_event(Event::OrderTaken(who, order_id, order));
                    Ok(())
                })
            })?;
            Ok(().into())
        }

        #[pallet::weight(10_000 + T::DbWeight::get().writes(1))]
        fn cancel_order(origin: OriginFor<T>, order_id: T::OrderId) -> DispatchResultWithPostInfo {
            let who = ensure_signed(origin)?;

            Orders::<T>::try_mutate_exists(order_id, |order| -> DispatchResult {
                let order = order
                    .take()
                    .ok_or(Error::<T>::InvalidOrderId)?
                    .ok_or(Error::<T>::OrderIsNone)?;

                ensure!(order.owner == who, Error::<T>::NotOwner);

                Self::deposit_event(Event::OrderCancelled(order_id));
                Ok(())
            })?;

            Ok(().into())
        }
    }
}
