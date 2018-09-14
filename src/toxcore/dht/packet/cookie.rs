/*! Cookie struct
*/

use nom::be_u64;

use std::io::{Error, ErrorKind};
use std::time::SystemTime;

use toxcore::binary_io::*;
use toxcore::crypto_core::*;
use toxcore::time::*;

/// Number of seconds that generated cookie is valid
pub const COOKIE_TIMEOUT: u64 = 15;

/** Cookie is a struct that holds two public keys of a node: long term key and
short term DHT key.

When Alice establishes `net_crypto` connection with Bob she sends
`CookieRequest` packet to Bob with her public keys and receives encrypted
`Cookie` with these keys from `CookieResponse` packet. When Alice obtains a
`Cookie` she uses it to send `CryptoHandshake` packet. This packet will contain
received from Bob cookie and new `Cookie` generated by Alice. Then Bob checks
his `Coocke` and uses `Cookie` from Alice to send `CryptoHandshake` packet to
her.

Only node that encrypted a `Cookie` can decrypt it so when node gets
`CryptoHandshake` packet with `Cookie` it can check that the sender of this
packet received a cookie response.

Cookie also contains the time when it was generated. It's considered invalid
after 15 seconds have elapsed since the moment of generation.

Serialized form:

Length | Content
------ | ------
`8`    | Cookie timestamp
`32`   | Long term `PublicKey`
`32`   | DHT `PublicKey`

*/
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Cookie {
    /// Time when this cookie was generated
    pub time: u64,
    /// Long term `PublicKey`
    pub real_pk: PublicKey,
    /// DHT `PublicKey`
    pub dht_pk: PublicKey,
}

impl Cookie {
    /// Create new `Cookie`
    pub fn new(real_pk: PublicKey, dht_pk: PublicKey) -> Cookie {
        Cookie {
            time: unix_time(SystemTime::now()),
            real_pk,
            dht_pk,
        }
    }

    /** Check if this cookie is timed out.

    Cookie considered timed out after 15 seconds since it was created.

    */
    pub fn is_timed_out(&self) -> bool {
        self.time + COOKIE_TIMEOUT < unix_time(SystemTime::now())
    }
}

impl FromBytes for Cookie {
    named!(from_bytes<Cookie>, do_parse!(
        time: be_u64 >>
        real_pk: call!(PublicKey::from_bytes) >>
        dht_pk: call!(PublicKey::from_bytes) >>
        eof!() >>
        (Cookie { time, real_pk, dht_pk })
    ));
}

impl ToBytes for Cookie {
    fn to_bytes<'a>(&self, buf: (&'a mut [u8], usize)) -> Result<(&'a mut [u8], usize), GenError> {
        do_gen!(buf,
            gen_be_u64!(self.time) >>
            gen_slice!(self.real_pk.as_ref()) >>
            gen_slice!(self.dht_pk.as_ref())
        )
    }
}

/** Encrypted with symmetric key `Cookie`.

Serialized form:

Length | Content
------ | ------
`24`   | Nonce
`88`   | Payload

*/
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EncryptedCookie {
    /// Nonce for the current encrypted payload
    pub nonce: secretbox::Nonce,
    /// Encrypted payload
    pub payload: Vec<u8>,
}

impl FromBytes for EncryptedCookie {
    named!(from_bytes<EncryptedCookie>, do_parse!(
        nonce: call!(secretbox::Nonce::from_bytes) >>
        payload: take!(88) >>
        (EncryptedCookie { nonce, payload: payload.to_vec() })
    ));
}

impl ToBytes for EncryptedCookie {
    fn to_bytes<'a>(&self, buf: (&'a mut [u8], usize)) -> Result<(&'a mut [u8], usize), GenError> {
        do_gen!(buf,
            gen_slice!(self.nonce.as_ref()) >>
            gen_slice!(self.payload.as_slice())
        )
    }
}

impl EncryptedCookie {
    /// Create `EncryptedCookie` from `Cookie` encrypting it with `symmetric_key`
    pub fn new(symmetric_key: &secretbox::Key, payload: &Cookie) -> EncryptedCookie {
        let nonce = secretbox::gen_nonce();
        let mut buf = [0; 72];
        let (_, size) = payload.to_bytes((&mut buf, 0)).unwrap();
        let payload = secretbox::seal(&buf[..size], &nonce, symmetric_key);

        EncryptedCookie {
            nonce,
            payload,
        }
    }
    /** Decrypt payload with symmetric key and try to parse it as `Cookie`.

    Returns `Error` in case of failure:

    - fails to decrypt
    - fails to parse `Cookie`
    */
    pub fn get_payload(&self, symmetric_key: &secretbox::Key) -> Result<Cookie, Error> {
        let decrypted = secretbox::open(&self.payload, &self.nonce, symmetric_key)
            .map_err(|()| {
                debug!("Decrypting Cookie failed!");
                Error::new(ErrorKind::Other, "Cookie decrypt error.")
            })?;
        match Cookie::from_bytes(&decrypted) {
            IResult::Incomplete(e) => {
                debug!(target: "Dht", "Cookie return deserialize error: {:?}", e);
                Err(Error::new(ErrorKind::Other,
                    format!("Cookie return deserialize error: {:?}", e)))
            },
            IResult::Error(e) => {
                debug!(target: "Dht", "Cookie return deserialize error: {:?}", e);
                Err(Error::new(ErrorKind::Other,
                    format!("Cookie return deserialize error: {:?}", e)))
            },
            IResult::Done(_, payload) => {
                Ok(payload)
            }
        }
    }
    /// Calculate SHA512 hash of encrypted cookie together with nonce
    pub fn hash(&self) -> sha512::Digest {
        let mut buf = [0; 112];
        let (_, size) = self.to_bytes((&mut buf, 0)).unwrap();
        sha512::hash(&buf[..size])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    encode_decode_test!(
        cookie_encode_decode,
        Cookie {
            time: 12345,
            real_pk: gen_keypair().0,
            dht_pk: gen_keypair().0,
        }
    );

    encode_decode_test!(
        encrypted_cookie_encode_decode,
        EncryptedCookie {
            nonce: secretbox::gen_nonce(),
            payload: vec![42; 88],
        }
    );

    #[test]
    fn cookie_encrypt_decrypt() {
        let symmetric_key = secretbox::gen_key();
        let payload = Cookie::new(gen_keypair().0, gen_keypair().0);
        // encode payload with symmetric key
        let encrypted_cookie = EncryptedCookie::new(&symmetric_key, &payload);
        // decode payload with symmetric key
        let decoded_payload = encrypted_cookie.get_payload(&symmetric_key).unwrap();
        // payloads should be equal
        assert_eq!(decoded_payload, payload);
    }

    #[test]
    fn cookie_encrypt_decrypt_invalid_key() {
        let symmetric_key = secretbox::gen_key();
        let eve_symmetric_key = secretbox::gen_key();
        let payload = Cookie::new(gen_keypair().0, gen_keypair().0);
        // encode payload with symmetric key
        let encrypted_cookie = EncryptedCookie::new(&symmetric_key, &payload);
        // try to decode payload with eve's symmetric key
        let decoded_payload = encrypted_cookie.get_payload(&eve_symmetric_key);
        assert!(decoded_payload.is_err());
    }

    #[test]
    fn cookie_encrypt_decrypt_invalid() {
        let symmetric_key = secretbox::gen_key();
        let nonce = secretbox::gen_nonce();
        // Try long invalid array
        let invalid_payload = [42; 123];
        let invalid_payload_encoded = secretbox::seal(&invalid_payload, &nonce, &symmetric_key);
        let invalid_encrypted_cookie = EncryptedCookie {
            nonce,
            payload: invalid_payload_encoded
        };
        let decoded_payload = invalid_encrypted_cookie.get_payload(&symmetric_key);
        assert!(decoded_payload.is_err());
        // Try short incomplete array
        let invalid_payload = [];
        let invalid_payload_encoded = secretbox::seal(&invalid_payload, &nonce, &symmetric_key);
        let invalid_encrypted_cookie = EncryptedCookie {
            nonce,
            payload: invalid_payload_encoded
        };
        let decoded_payload = invalid_encrypted_cookie.get_payload(&symmetric_key);
        assert!(decoded_payload.is_err());
    }

    #[test]
    fn cookie_timed_out() {
        let mut cookie = Cookie::new(gen_keypair().0, gen_keypair().0);
        assert!(!cookie.is_timed_out());
        cookie.time -= COOKIE_TIMEOUT + 1;
        assert!(cookie.is_timed_out());
    }

    #[test]
    fn hash_depends_on_all_fields() {
        let nonce = secretbox::gen_nonce();
        let payload = vec![42; 88];
        let cookie = EncryptedCookie {
            nonce,
            payload: payload.clone()
        };

        let cookie_1 = EncryptedCookie {
            nonce,
            payload: vec![43; 88]
        };
        let cookie_2 = EncryptedCookie {
            nonce: secretbox::gen_nonce(),
            payload
        };

        assert!(cookie.hash() != cookie_1.hash());
        assert!(cookie.hash() != cookie_2.hash());
    }
}
