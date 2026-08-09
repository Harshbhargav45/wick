import { AnchorProvider, Program, Wallet } from "@coral-xyz/anchor";
import { Connection, Keypair, PublicKey } from "@solana/web3.js";
import { Buffer as IsomorphicBuffer } from "node:buffer";
import { parseAccumulatorUpdateData } from "@pythnetwork/price-service-sdk";
import { IDL as ReceiverIdl } from "@pythnetwork/pyth-solana-receiver/idl/pyth_solana_receiver";
import { IDL as WormholeIdl } from "@pythnetwork/pyth-solana-receiver/idl/wormhole_core_bridge_solana";
import {
  getConfigPda,
  getGuardianSetPda,
  getRandomTreasuryId,
  getTreasuryPda,
} from "@pythnetwork/pyth-solana-receiver/address";
import { config, crankerKeypair } from "./config.mjs";

const VAA_START = 46;
const VAA_SPLIT_INDEX = 721;

export function getGuardianSetIndex(vaa) {
  return vaa.readUInt32BE(1);
}

export function trimSignatures(vaa, n = 5) {
  const currentNumSignatures = vaa[5];
  if (n >= currentNumSignatures) return vaa;
  const VAA_SIGNATURE_SIZE = 66;
  const trimmed = IsomorphicBuffer.concat([
    vaa.subarray(0, 6 + n * VAA_SIGNATURE_SIZE),
    vaa.subarray(6 + currentNumSignatures * VAA_SIGNATURE_SIZE),
  ]);
  trimmed[5] = n;
  return trimmed;
}

export async function readWormholeProgramId(connection, receiverProgramId) {
  const configPda = getConfigPda(receiverProgramId);
  const info = await connection.getAccountInfo(configPda);
  if (!info) {
    throw new Error(`receiver config PDA missing: ${configPda.toBase58()}`);
  }
  // Config layout: [disc 8][gov 32][option tag 1][wormhole 32]
  return new PublicKey(info.data.slice(41, 73));
}

export async function createReceiverPrograms() {
  const connection = new Connection(config.rpc, "confirmed");
  const wallet = new Wallet(crankerKeypair());
  const provider = new AnchorProvider(connection, wallet, {
    commitment: "confirmed",
  });
  const receiver = new Program(ReceiverIdl, config.receiverProgramId, provider);
  // The on-chain config PDA is the source of truth for the Wormhole program id;
  // the SDK's DEFAULT_WORMHOLE_PROGRAM_ID constant is wrong on this cluster.
  const wormholeProgramId = await readWormholeProgramId(
    connection,
    config.receiverProgramId
  );
  const wormhole = new Program(WormholeIdl, wormholeProgramId, provider);
  return { connection, provider, receiver, wormhole, wormholeProgramId };
}

async function buildEncodedVaaInstructions(wormhole, vaa) {
  const encodedVaaKeypair = new Keypair();
  const encodedVaaSize = vaa.length + VAA_START;
  const create = await wormhole.account.encodedVaa.createInstruction(
    encodedVaaKeypair,
    encodedVaaSize
  );
  const init = await wormhole.methods
    .initEncodedVaa()
    .accounts({ encodedVaa: encodedVaaKeypair.publicKey })
    .instruction();

  const write1 = await wormhole.methods
    .writeEncodedVaa({ data: vaa.subarray(0, VAA_SPLIT_INDEX), index: 0 })
    .accounts({ draftVaa: encodedVaaKeypair.publicKey })
    .instruction();

  const tail = [];
  if (vaa.length > VAA_SPLIT_INDEX) {
    tail.push(
      await wormhole.methods
        .writeEncodedVaa({
          data: vaa.subarray(VAA_SPLIT_INDEX),
          index: VAA_SPLIT_INDEX,
        })
        .accounts({ draftVaa: encodedVaaKeypair.publicKey })
        .instruction()
    );
  }
  const verify = await wormhole.methods
    .verifyEncodedVaaV1()
    .accounts({
      draftVaa: encodedVaaKeypair.publicKey,
      guardianSet: getGuardianSetPda(getGuardianSetIndex(vaa), wormhole.programId),
    })
    .instruction();
  tail.push(verify);

  const close = await wormhole.methods
    .closeEncodedVaa()
    .accounts({ encodedVaa: encodedVaaKeypair.publicKey })
    .instruction();

  return {
    encodedVaa: encodedVaaKeypair.publicKey,
    encodedVaaSigner: encodedVaaKeypair,
    initTx: [create, init, write1],
    verifyTx: tail,
    close,
  };
}

export async function buildPostUpdateInstructions(vaaBase64) {
  const { receiver, wormhole } = await createReceiverPrograms();
  const accumulator = parseAccumulatorUpdateData(
    IsomorphicBuffer.from(vaaBase64, "base64")
  );
  const vaa = accumulator.vaa;

  const { encodedVaa, encodedVaaSigner, initTx, verifyTx, close } =
    await buildEncodedVaaInstructions(wormhole, vaa);

  const treasuryId = getRandomTreasuryId();
  const update = accumulator.updates[0];
  const priceUpdateKeypair = new Keypair();

  const postUpdateIx = await receiver.methods
    .postUpdate({
      merklePriceUpdate: update,
      treasuryId,
    })
    .accounts({
      config: getConfigPda(receiver.programId),
      encodedVaa,
      priceUpdateAccount: priceUpdateKeypair.publicKey,
      treasury: getTreasuryPda(treasuryId, receiver.programId),
    })
    .instruction();

  return {
    initTx,
    verifyTx,
    postUpdateIx,
    encodedVaa,
    encodedVaaSigner,
    priceUpdateAccount: priceUpdateKeypair.publicKey,
    priceUpdateSigner: priceUpdateKeypair,
    close,
  };
}
