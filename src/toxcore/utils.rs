/*! Common utility functions
*/

use crate::toxcore::crypto_core::*;

/// Generate non-zero ping_id
pub fn gen_ping_id() -> u64 {
    let mut ping_id = 0;
    while ping_id == 0 {
        ping_id = random_u64();
    }
    ping_id
}
