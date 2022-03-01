
use {
    crate::{
        transfer_proof::TransferData,
    },
    bytemuck::{Pod, Zeroable},
    num_derive::{FromPrimitive, ToPrimitive},
    num_traits::{FromPrimitive, ToPrimitive},
    solana_program::{
        instruction::{AccountMeta, Instruction},
        program_error::ProgramError,
        pubkey::Pubkey,
        sysvar,
    },
    crate::{
        zk_token_elgamal,
    },
};

#[cfg(not(target_arch = "bpf"))]
use {
    crate::equality_proof,
    solana_program::{
        system_instruction,
    },
    solana_sdk::signer::Signer,
    std::convert::TryInto,
};

#[derive(Clone, Copy, Pod, Zeroable)]
#[repr(C)]
pub struct ConfigureMetadataData {
    /// The ElGamal public key associated with the owner public key and this NFT mint.
    /// NB: this is not checked on initialization but should be the canonical one for compatibility
    pub elgamal_pk: zk_token_elgamal::pod::ElGamalPubkey,

    /// AES Cipher key for the encrypted asset. This should already by encrypted with `elgamal_pk`
    ///
    /// This is chunked because the version of ElGamal we're using is slow in decrypting so we must
    /// keep the encrypted values small (<32 bits).
    pub encrypted_cipher_key: zk_token_elgamal::pod::ElGamalCiphertext,

    /// The URI of the encrypted asset
    pub uri: crate::state::URI,
}

#[derive(Clone, Copy, Pod, Zeroable)]
#[repr(C)]
pub struct TransferChunkData {
    /// Transfer Data (proof statement and masking factors)
    pub transfer: TransferData,
}

#[derive(Clone, Copy, Pod, Zeroable)]
#[repr(C)]
pub struct TransferChunkSlowData {
    /// Transfer Data (proof statement and masking factors)
    pub transfer: TransferData,
}

#[derive(Clone, Copy, Debug, FromPrimitive, ToPrimitive)]
#[repr(u8)]
pub enum StealthInstruction {
    /// Configures private metadata for an NFT
    ///
    /// Accounts expected by this instruction:
    ///
    ///   0. `[writeable,signer]` Payer
    ///   1. `[]` The SPL Token mint account of the NFT
    ///   2. `[]` The SPL Metadata account. Must be mutable
    ///   3. `[signer]` The update authority for the SPL Metadata
    ///   4. `[writeable]` Stealth PDA
    ///   5. `[]` Metadata program
    ///   6. `[]` System program
    ///   7. `[]` Rent sysvar
    ///
    /// Data expected by this instruction:
    ///   ConfigureMetadataData
    ///
    ConfigureMetadata,

    /// Initialise transfer state for private metadata
    ///
    /// Accounts expected by this instruction:
    ///
    ///   0. `[writeable,signer]` The owner of the NFT
    ///   1. `[]` The SPL Token mint account of the NFT
    ///   2. `[]` The SPL Token account holding the NFT
    ///   3. `[writable]` Stealth PDA
    ///   4. `[]` Recipient wallet
    ///   5. `[]` Recipient elgamal pubkey PDA
    ///   6. `[writable]` Transfer buffer PDA. Will hold CipherKeyTransferBuffer
    ///   7. `[]` System program
    ///   8. `[]` Rent sysvar
    ///
    /// Data expected by this instruction:
    ///
    InitTransfer,

    /// Finalise transfer state for private metadata and swap cipher texts
    ///
    /// Accounts expected by this instruction:
    ///
    ///   0. `[writeable,signer]` Authority. Must be the authority on the transfer buffer
    ///   1. `[]` Stealth PDA
    ///   2. `[writable]` Transfer buffer program account
    ///   3. `[]` System program
    ///
    FiniTransfer,

    /// Validate encrypted cipher key chunk. NB: this will not run within compute limits without
    /// syscall support for crypto instructions.
    ///
    /// Accounts expected by this instruction:
    ///
    ///   0. `[writeable,signer]` Authority. Must be the authority on the transfer buffer
    ///   1. `[]` Stealth PDA
    ///   2. `[writable]` Transfer buffer program account
    ///   3. `[]` System program
    ///
    /// Data expected by this instruction:
    ///   TransferChunkData
    ///
    TransferChunk,

    /// Validate encrypted cipher key chunk through a manual DSL cranked instruction.
    ///
    /// Accounts expected by this instruction:
    ///
    ///   0. `[writeable,signer]` Authority. Must be the authority on the transfer buffer
    ///   1. `[]` Stealth PDA
    ///   2. `[writable]` Transfer buffer program account
    ///   3. `[]` Instruction buffer. Must match Header + equality_proof::DSL_INSTRUCTION_BYTES
    ///   4. `[]` Input buffer. Must have the appropriate proof points and scalars
    ///   5. `[]` Compute buffer. Must match the instruction + input buffers and have been cranked
    ///      for all DSL instructions
    ///   6. `[]` System program
    ///
    /// Data expected by this instruction:
    ///   TransferChunkSlowData
    ///
    TransferChunkSlow,

    /// Write an elgamal pubkey into the associated buffer for this wallet and mint
    ///
    /// Accounts expected by this instruction:
    ///
    ///   0. `[writeable,signer]` Wallet to publish for
    ///   1. `[]` The SPL Token mint account of the NFT
    ///   2. `[writable]` The elgamal pubkey PDA
    ///   3. `[]` System program
    ///   4. `[]` Rent sysvar
    ///
    /// Data expected by this instruction:
    ///   elgamal_pk: The recipients elgamal public-key
    ///
    PublishElgamalPubkey,

    /// Close the associated elgamal pubkey buffer for this wallet and mint
    ///
    /// Accounts expected by this instruction:
    ///
    ///   0. `[writeable,signer]` Wallet to close buffer for
    ///   1. `[]` The SPL Token mint account of the NFT
    ///   2. `[writable]` The elgamal pubkey PDA
    ///   3. `[]` System program
    ///
    /// Data expected by this instruction:
    ///
    CloseElgamalPubkey,
}

pub fn decode_instruction_type(
    input: &[u8]
) -> Result<StealthInstruction, ProgramError> {
    if input.is_empty() {
        Err(ProgramError::InvalidInstructionData)
    } else {
        FromPrimitive::from_u8(input[0]).ok_or(ProgramError::InvalidInstructionData)
    }
}

pub fn decode_instruction_data<T: Pod>(
    input: &[u8]
) -> Result<&T, ProgramError> {
    if input.len() < 2 {
        Err(ProgramError::InvalidInstructionData)
    } else {
        pod_from_bytes(&input[1..]).ok_or(ProgramError::InvalidArgument)
    }
}

/// Convert a slice into a `Pod` (zero copy)
pub fn pod_from_bytes<T: Pod>(bytes: &[u8]) -> Option<&T> {
    bytemuck::try_from_bytes(bytes).ok()
}

pub fn get_metadata_address(mint: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            mpl_token_metadata::state::PREFIX.as_bytes(),
            mpl_token_metadata::ID.as_ref(),
            mint.as_ref(),
        ],
        &mpl_token_metadata::ID,
    )
}

pub fn get_stealth_address(mint: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            crate::state::PREFIX.as_bytes(),
            mint.as_ref(),
        ],
        &crate::ID,
    )
}

pub fn get_elgamal_pubkey_address(
    wallet: &Pubkey,
    mint: &Pubkey,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            crate::state::PREFIX.as_bytes(),
            wallet.as_ref(),
            mint.as_ref(),
        ],
        &crate::ID,
    )
}

pub fn get_transfer_buffer_address(
    wallet: &Pubkey,
    mint: &Pubkey,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            crate::state::TRANSFER.as_bytes(),
            wallet.as_ref(),
            mint.as_ref(),
        ],
        &crate::ID,
    )
}

pub fn encode_instruction<T: Pod>(
    accounts: Vec<AccountMeta>,
    instruction_type: StealthInstruction,
    instruction_data: &T,
) -> Instruction {
    let mut data = vec![ToPrimitive::to_u8(&instruction_type).unwrap()];
    data.extend_from_slice(bytemuck::bytes_of(instruction_data));
    Instruction {
        program_id: crate::ID,
        accounts,
        data,
    }
}

#[cfg(not(target_arch = "bpf"))]
pub fn configure_metadata(
    payer: Pubkey,
    mint: Pubkey,
    elgamal_pk: zk_token_elgamal::pod::ElGamalPubkey,
    encrypted_cipher_key: &zk_token_elgamal::pod::ElGamalCiphertext,
    uri: &[u8],
) -> Instruction {
    let accounts = vec![
        AccountMeta::new(payer, true),
        AccountMeta::new_readonly(mint, false),
        AccountMeta::new(get_metadata_address(&mint).0, false),
        AccountMeta::new_readonly(payer, true),
        AccountMeta::new(get_stealth_address(&mint).0, false),
        AccountMeta::new_readonly(solana_program::system_program::id(), false),
        AccountMeta::new_readonly(sysvar::rent::id(), false),
    ];

    let mut data = ConfigureMetadataData::zeroed();
    data.elgamal_pk = elgamal_pk;
    data.encrypted_cipher_key = *encrypted_cipher_key;
    data.uri.0[..uri.len()].copy_from_slice(uri);

    encode_instruction(
        accounts,
        StealthInstruction::ConfigureMetadata,
        &data,
    )
}

pub fn init_transfer(
    payer: &Pubkey,
    mint: &Pubkey,
    recipient: &Pubkey,
) -> Instruction {
    let accounts = vec![
        AccountMeta::new(*payer, true),
        AccountMeta::new_readonly(*mint, false),
        AccountMeta::new_readonly(
            spl_associated_token_account::get_associated_token_address(payer, mint),
            false,
        ),
        AccountMeta::new_readonly(get_stealth_address(mint).0, false),
        AccountMeta::new_readonly(*recipient, false),
        AccountMeta::new_readonly(get_elgamal_pubkey_address(recipient, mint).0, false),
        AccountMeta::new(get_transfer_buffer_address(recipient, mint).0, false),
        AccountMeta::new_readonly(solana_program::system_program::id(), false),
        AccountMeta::new_readonly(sysvar::rent::id(), false),
    ];

    encode_instruction(
        accounts,
        StealthInstruction::InitTransfer,
        &(),
    )
}

pub fn fini_transfer(
    payer: Pubkey,
    mint: Pubkey,
    transfer_buffer: Pubkey,
) -> Instruction {
    let accounts = vec![
        AccountMeta::new(payer, true),
        AccountMeta::new(get_stealth_address(&mint).0, false),
        AccountMeta::new(transfer_buffer, false),
        AccountMeta::new_readonly(solana_program::system_program::id(), false),
    ];

    encode_instruction(
        accounts,
        StealthInstruction::FiniTransfer,
        &(),
    )
}

#[cfg(not(target_arch = "bpf"))]
pub fn transfer_chunk(
    payer: Pubkey,
    mint: Pubkey,
    transfer_buffer: Pubkey,
    data: TransferChunkData,
) -> Instruction {
    let accounts = vec![
        AccountMeta::new(payer, true),
        AccountMeta::new_readonly(get_stealth_address(&mint).0, false),
        AccountMeta::new(transfer_buffer, false),
        AccountMeta::new_readonly(solana_program::system_program::id(), false),
    ];

    encode_instruction(
        accounts,
        StealthInstruction::TransferChunk,
        &data,
    )
}

#[cfg(not(target_arch = "bpf"))]
pub fn transfer_chunk_slow(
    payer: Pubkey,
    mint: Pubkey,
    transfer_buffer: Pubkey,
    instruction_buffer: Pubkey,
    input_buffer: Pubkey,
    compute_buffer: Pubkey,
    data: TransferChunkSlowData,
) -> Instruction {
    let accounts = vec![
        AccountMeta::new(payer, true),
        AccountMeta::new_readonly(get_stealth_address(&mint).0, false),
        AccountMeta::new(transfer_buffer, false),
        AccountMeta::new_readonly(instruction_buffer, false),
        AccountMeta::new_readonly(input_buffer, false),
        AccountMeta::new_readonly(compute_buffer, false),
        AccountMeta::new_readonly(solana_program::system_program::id(), false),
    ];

    encode_instruction(
        accounts,
        StealthInstruction::TransferChunkSlow,
        &data,
    )
}

#[cfg(not(target_arch = "bpf"))]
pub fn publish_elgamal_pubkey(
    payer: &Pubkey,
    mint: &Pubkey,
    elgamal_pk: zk_token_elgamal::pod::ElGamalPubkey,
) -> Instruction {
    let accounts = vec![
        AccountMeta::new(*payer, true),
        AccountMeta::new_readonly(*mint, false),
        AccountMeta::new(get_elgamal_pubkey_address(&payer, &mint).0, false),
        AccountMeta::new_readonly(solana_program::system_program::id(), false),
        AccountMeta::new_readonly(sysvar::rent::id(), false),
    ];

    encode_instruction(
        accounts,
        StealthInstruction::PublishElgamalPubkey,
        &elgamal_pk,
    )
}

#[cfg(not(target_arch = "bpf"))]
pub fn close_elgamal_pubkey(
    payer: &Pubkey,
    mint: &Pubkey,
) -> Instruction {
    let accounts = vec![
        AccountMeta::new(*payer, true),
        AccountMeta::new_readonly(*mint, false),
        AccountMeta::new(get_elgamal_pubkey_address(&payer, &mint).0, false),
        AccountMeta::new_readonly(solana_program::system_program::id(), false),
    ];

    encode_instruction(
        accounts,
        StealthInstruction::CloseElgamalPubkey,
        &(),
    )
}

#[cfg(not(target_arch = "bpf"))]
pub struct InstructionsAndSigners<'a> {
    pub instructions: Vec<Instruction>,
    pub signers: Vec<&'a dyn Signer>,
}

#[cfg(not(target_arch = "bpf"))]
pub fn populate_transfer_proof_dsl<'a, F>(
    payer: &'a dyn Signer,
    instruction_buffer: &'a dyn Signer,
    minimum_rent_balance: F,
) -> Vec<InstructionsAndSigners<'a>>
    where F: Fn(usize) -> u64,
{
    use curve25519_dalek_onchain::instruction as dalek;

    let dsl_len = equality_proof::DSL_INSTRUCTION_BYTES.len();
    let instruction_buffer_len = dalek::HEADER_SIZE + dsl_len;

    let mut ret = vec![];

    ret.push(InstructionsAndSigners{
        instructions: vec![
            system_instruction::create_account(
                &payer.pubkey(),
                &instruction_buffer.pubkey(),
                minimum_rent_balance(instruction_buffer_len),
                instruction_buffer_len as u64,
                &curve25519_dalek_onchain::id(),
            ),
            dalek::initialize_buffer(
                instruction_buffer.pubkey(),
                payer.pubkey(),
                dalek::Key::InstructionBufferV1,
                vec![],
            ),
        ],
        signers: vec![payer, instruction_buffer],
    });

    // write the instructions
    let mut dsl_idx = 0;
    let dsl_chunk = 800;
    loop {
        let mut instructions = vec![];
        let end = (dsl_idx+dsl_chunk).min(dsl_len);
        let done = end == dsl_len;
        instructions.push(
            dalek::write_bytes(
                instruction_buffer.pubkey(),
                payer.pubkey(),
                (dalek::HEADER_SIZE + dsl_idx).try_into().unwrap(),
                done,
                &equality_proof::DSL_INSTRUCTION_BYTES[dsl_idx..end],
            )
        );
        ret.push(InstructionsAndSigners{
            instructions,
            signers: vec![payer],
        });
        if done {
            break;
        } else {
            dsl_idx = end;
        }
    }

    ret
}

#[cfg(not(target_arch = "bpf"))]
#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct InstructionsAndSignerPubkeys {
    pub instructions: Vec<Instruction>,
    pub signers: Vec<Pubkey>,
}

// Returns a list of transaction instructions that can be sent to build the zk proof state used in
// a `transfer_chunk_slow`. These instructions assume that the instruction DSL has already been
// populated with `populate_transfer_proof_dsl`
#[cfg(not(target_arch = "bpf"))]
pub fn transfer_chunk_slow_proof<F>(
    payer: &Pubkey,
    instruction_buffer: &Pubkey,
    input_buffer: &Pubkey,
    compute_buffer: &Pubkey,
    transfer: &TransferData,
    minimum_rent_balance: F,
) -> Result<Vec<InstructionsAndSignerPubkeys>, Box<dyn std::error::Error>>
    where F: Fn(usize) -> u64,
{
    use crate::transcript::TranscriptProtocol;
    use crate::transfer_proof::TransferProof;
    use curve25519_dalek::scalar::Scalar;
    use curve25519_dalek_onchain::instruction as dalek;
    use curve25519_dalek_onchain::{window::LookupTable, edwards::ProjectiveNielsPoint};
    use curve25519_dalek_onchain::scalar::Scalar as OScalar;

    let equality_proof = equality_proof::EqualityProof::from_bytes(
        &transfer.proof.equality_proof.0)?;

    let points = [
        // statement inputs
        transfer.transfer_public_keys.src_pubkey.0,
        equality_proof::COMPRESSED_H,
        equality_proof.Y_0.0,

        transfer.transfer_public_keys.dst_pubkey.0,
        transfer.dst_cipher_key_chunk_ct.0[32..].try_into()?,
        equality_proof.Y_1.0,

        transfer.dst_cipher_key_chunk_ct.0[..32].try_into()?,
        transfer.src_cipher_key_chunk_ct.0[..32].try_into()?,
        transfer.src_cipher_key_chunk_ct.0[32..].try_into()?,
        equality_proof::COMPRESSED_H,
        equality_proof.Y_2.0,
    ];

    let mut transcript = TransferProof::transcript_new();
    TransferProof::build_transcript(
        &transfer.src_cipher_key_chunk_ct,
        &transfer.dst_cipher_key_chunk_ct,
        &transfer.transfer_public_keys,
        &mut transcript,
    )?;

    equality_proof::EqualityProof::build_transcript(
        &equality_proof,
        &mut transcript,
    )?;

    let challenge_c = transcript.challenge_scalar(b"c");
    let challenge_w = transcript.challenge_scalar(b"w");
    let challenge_ww = challenge_w  * challenge_w;

    // the equality_proof points are normal 'Scalar' but the DSL crank expects it's version of the
    // type
    let scalars = vec![
         equality_proof.sh_1,
         -challenge_c,
         -Scalar::one(),

         &challenge_w * &equality_proof.rh_2,
         -challenge_w * challenge_c,
         -challenge_w * Scalar::one(),

         challenge_ww * challenge_c,
         -challenge_ww * challenge_c,
         challenge_ww * equality_proof.sh_1,
         -challenge_ww * equality_proof.rh_2,
         -challenge_ww * Scalar::one(),
    ]
        .iter()
        .map(|s| OScalar::from_canonical_bytes(s.bytes))
        .collect::<Option<Vec<_>>>()
        .ok_or("failed to canonicalise equality proof scalars")?;

    assert_eq!(points.len(), scalars.len());

    let input_buffer_len = dalek::HEADER_SIZE + points.len() * 32 * 3 + 128;

    let compute_buffer_len =
        dalek::HEADER_SIZE
        + 3 * 32 * 4                 // 3 proof groups
        + 32 * 12                    // decompression space
        + 32 * scalars.len()         // scalars
        + LookupTable::<ProjectiveNielsPoint>::TABLE_SIZE * points.len()  // point lookup tables
        ;

    let mut ret = vec![];

    ret.push(InstructionsAndSignerPubkeys{
        instructions: vec![
            system_instruction::create_account(
                payer,
                input_buffer,
                minimum_rent_balance(input_buffer_len),
                input_buffer_len as u64,
                &curve25519_dalek_onchain::id(),
            ),
            system_instruction::create_account(
                payer,
                compute_buffer,
                minimum_rent_balance(compute_buffer_len),
                compute_buffer_len as u64,
                &curve25519_dalek_onchain::id(),
            ),
            dalek::initialize_buffer(
                *input_buffer,
                *payer,
                dalek::Key::InputBufferV1,
                vec![],
            ),
            dalek::initialize_buffer(
                *compute_buffer,
                *payer,
                dalek::Key::ComputeBufferV1,
                vec![*instruction_buffer, *input_buffer],
            ),
        ].into_iter().chain(
            dalek::write_input_scalars_and_identity(
                *input_buffer,
                *payer,
                scalars.as_slice(),
            ),
        ).collect(),
        signers: vec![*payer, *input_buffer, *compute_buffer],
    });

    ret.push(InstructionsAndSignerPubkeys{
        instructions: dalek::write_input_points(
            *input_buffer,
            *payer,
            &points,
        ).ok_or("Invalid ristretto input points")?
         .into_iter().chain(
            dalek::finalize_buffer(*input_buffer, *payer),
        ).collect(),
        signers: vec![*payer],
    });

    let crank = dalek::crank_compute(
        *instruction_buffer,
        *input_buffer,
        *compute_buffer,
    );

    let mut current = 0;
    let mut crank_transactions = 0;

    let mut add_crank_batch = |count| {
        let mut instructions = vec![
            solana_sdk::compute_budget::ComputeBudgetInstruction::request_units(1_000_000),
            dalek::noop(crank_transactions),
        ];
        instructions.extend_from_slice(&vec![crank.clone(); count]);
        current += count;
        ret.push(InstructionsAndSignerPubkeys{
            instructions,
            signers: vec![*payer],
        });
        crank_transactions += 1;
    };

    // 11 proof inputs, 3 inputs for each. each takes ~140k compute so do 7 and then pack the next
    // 4 with scalar / identity copies
    add_crank_batch(7 * 3);
    add_crank_batch(4 * 3 + 11 + 1);

    // then we have 64 multiplication cranks each is ~200k compute so we can pack ~5 * 12 + 4
    for _f in 0..12 {
        add_crank_batch(5);
    }
    add_crank_batch(4);

    assert_eq!(current, equality_proof::DSL_INSTRUCTION_COUNT);
    assert_eq!(crank_transactions, 15);

    Ok(ret)
}