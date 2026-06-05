use ic_vetkeys::{TransportSecretKey, DerivedPublicKey, EncryptedVetKey, IbeCiphertext, IbeIdentity, IbeSeed};
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let seed = [42u8; 32];
    let tsk = TransportSecretKey::from_seed(seed.to_vec()).expect("tsk");
    if a[1] == "gen" {
        // print the 48-byte transport public key as candid blob escapes \xx
        let tpk = tsk.public_key();
        let s: String = tpk.iter().map(|b| format!("\\{:02x}", b)).collect();
        print!("{}", s);
        return;
    }
    // verify: master_pk_hex encrypted_key_hex input_hex
    let master = hex::decode(&a[2]).expect("master hex");
    let ek = hex::decode(&a[3]).expect("ek hex");
    let input = candid::Principal::from_text(&a[4]).expect("principal").as_slice().to_vec();
    let dpk = DerivedPublicKey::deserialize(&master).expect("dpk");
    let evk = EncryptedVetKey::deserialize(&ek).expect("evk");
    let vk = evk.decrypt_and_verify(&tsk, &dpk, &input).expect("decrypt_and_verify FAILED");
    // IBE round trip: encrypt a secret to this identity, decrypt with the vetKey
    let msg = b"motoview vetkeys secret";
    let id = IbeIdentity::from_bytes(&input);
    let iseed = IbeSeed::from_bytes(&[7u8; 32]).expect("seed");
    let ct = IbeCiphertext::encrypt(&dpk, &id, msg, &iseed);
    let ser = ct.serialize();
    let ct2 = IbeCiphertext::deserialize(&ser).expect("ct deser");
    let pt = ct2.decrypt(&vk).expect("ibe decrypt FAILED");
    assert_eq!(&pt, msg, "IBE plaintext mismatch");
    println!("ROUND_TRIP_OK master={} encrypted_key={} vetkey_sig={} ibe_ct={} plaintext_recovered={}",
        master.len(), ek.len(), vk.signature_bytes().len(), ser.len(), pt == msg);
}
