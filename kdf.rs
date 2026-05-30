// Stub: closed-source KDF module.
pub fn hkdf_expand(_secret: &[u8], _salt: &[u8], _info: &[u8], _out: &mut [u8]) {
    panic!("hydra-codingplan-crypto: kdf not available in open-source build")
}
