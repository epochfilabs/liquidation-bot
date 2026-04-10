use anyhow::Result;
use solana_sdk::pubkey::Pubkey;
use tokio::sync::mpsc;

use crate::config::AppConfig;
use crate::protocols::{self, ProtocolKind};

/// Represents a position account update from the gRPC stream.
pub enum PositionUpdate {
    AccountData {
        pubkey: Pubkey,
        owner_program: Pubkey,
        protocol: ProtocolKind,
        data: Vec<u8>,
    },
}

/// Subscribe to position account updates across all lending protocols.
pub async fn subscribe_all_protocols(
    config: &AppConfig,
) -> Result<mpsc::Receiver<PositionUpdate>> {
    let (tx, rx) = mpsc::channel(512);

    let grpc_url = config.grpc_url.clone();
    let grpc_token = config.grpc_token.clone();

    tokio::spawn(async move {
        if let Err(e) = run_subscription(grpc_url, grpc_token, tx).await {
            tracing::error!(error = %e, "gRPC subscription terminated");
        }
    });

    Ok(rx)
}

async fn run_subscription(
    grpc_url: String,
    grpc_token: Option<String>,
    tx: mpsc::Sender<PositionUpdate>,
) -> Result<()> {
    use yellowstone_grpc_client::GeyserGrpcClient;
    use yellowstone_grpc_proto::geyser::{
        SubscribeRequest, SubscribeRequestFilterAccounts,
    };
    use std::collections::HashMap;

    let mut client = GeyserGrpcClient::build_from_shared(grpc_url)?
        .x_token(grpc_token)?
        .connect()
        .await?;

    // Subscribe to accounts owned by ALL lending protocol programs
    let mut accounts_filter = HashMap::new();
    for (kind, program_id) in protocols::protocol_program_ids() {
        accounts_filter.insert(
            format!("{}_positions", kind),
            SubscribeRequestFilterAccounts {
                account: vec![],
                owner: vec![program_id.to_string()],
                filters: vec![],
                nonempty_txn_signature: None,
            },
        );
    }

    tracing::info!(
        protocols = accounts_filter.len(),
        "subscribing to {} lending protocol(s)",
        accounts_filter.len()
    );

    let request = SubscribeRequest {
        accounts: accounts_filter,
        ..Default::default()
    };

    let (_subscribe_tx, mut stream) = client.subscribe_with_request(Some(request)).await?;

    use tokio_stream::StreamExt;
    while let Some(msg_result) = stream.next().await {
        let msg = msg_result?;
        if let Some(update) = msg.update_oneof {
            use yellowstone_grpc_proto::geyser::subscribe_update::UpdateOneof;
            if let UpdateOneof::Account(account_update) = update {
                if let Some(account_info) = account_update.account {
                    let pubkey_bytes: [u8; 32] = account_info
                        .pubkey
                        .try_into()
                        .unwrap_or([0u8; 32]);
                    let pubkey = Pubkey::new_from_array(pubkey_bytes);

                    let owner_bytes: [u8; 32] = account_info
                        .owner
                        .try_into()
                        .unwrap_or([0u8; 32]);
                    let owner_program = Pubkey::new_from_array(owner_bytes);

                    // Identify which protocol this account belongs to
                    if let Some(protocol) = protocols::identify_protocol(&owner_program) {
                        let _ = tx
                            .send(PositionUpdate::AccountData {
                                pubkey,
                                owner_program,
                                protocol,
                                data: account_info.data,
                            })
                            .await;
                    }
                }
            }
        }
    }

    Ok(())
}
