use solana_address::Address;

#[test]
fn print_velocity_pdas() {
    let v = Address::from_str_const("vELoC1audYbSYVRXn1vPaV8Axoa9oU6BYmNGZZBDZ1P");
    let mk = |seeds: &[&[u8]], label: &str| {
        let (addr, bump) = Address::find_program_address(seeds, &v);
        println!("{label} = {addr} (bump {bump})");
    };
    mk(&[b"velocity_state"], "state");
    mk(&[b"perp_market", &0u16.to_le_bytes()], "perp_market_0");
    mk(&[b"spot_market", &0u16.to_le_bytes()], "spot_market_0");
    mk(&[b"spot_market_vault", &0u16.to_le_bytes()], "spot_vault_0");
    mk(
        &[b"insurance_fund_vault", &0u16.to_le_bytes()],
        "if_vault_0",
    );
    mk(&[b"drift_signer"], "drift_signer");
    mk(&[b"user_stats"], "user_stats");
}
