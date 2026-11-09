//! Asset forms of this game for the Moesekai site and its viewers, and the storage view.

#[cfg(feature = "costume")]
pub mod costume;
#[cfg(feature = "episode")]
pub mod episode;
pub mod roots;
#[cfg(feature = "scenario")]
pub mod scenario;
#[cfg(feature = "storage-view")]
pub mod storage_view;
#[cfg(feature = "virtual-live")]
pub mod virtual_live;
