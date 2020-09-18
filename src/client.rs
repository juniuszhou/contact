use crate::jsonrpc::client::HTTPClient;
use crate::jsonrpc::error::JsonRpcError;
use crate::types::*;
use crate::utils::maybe_get_optional_tx_info;
use clarity::Address as EthAddress;
use clarity::{abi::encode_tokens, abi::Token, PrivateKey as EthPrivateKey};
use deep_space::address::Address;
use deep_space::coin::Coin;
use deep_space::msg::{Msg, SendMsg, SetEthAddressMsg, ValsetConfirmMsg, ValsetRequestMsg};
use deep_space::private_key::PrivateKey;
use deep_space::stdfee::StdFee;
use deep_space::stdsignmsg::StdSignMsg;
use deep_space::transaction::Transaction;
use deep_space::transaction::TransactionSendType;
use deep_space::utils::bytes_to_hex_str;
use std::sync::Arc;
use std::time::Duration;

/// An instance of Contact Cosmos RPC Client.
#[derive(Clone)]
pub struct Contact {
    jsonrpc_client: Arc<Box<HTTPClient>>,
    timeout: Duration,
}

impl Contact {
    pub fn new(url: &str, timeout: Duration) -> Self {
        let mut url = url;
        if !url.ends_with('/') {
            url = url.trim_end_matches('/');
        }
        Self {
            jsonrpc_client: Arc::new(Box::new(HTTPClient::new(&url))),
            timeout,
        }
    }

    pub async fn get_latest_block(&self) -> Result<LatestBlockEndpointResponse, JsonRpcError> {
        let none: Option<bool> = None;
        self.jsonrpc_client
            .request_method("blocks/latest", none, self.timeout, None)
            .await
    }

    /// Gets account info for the provided Cosmos account using the accounts endpoint
    pub async fn get_account_info(
        &self,
        address: Address,
    ) -> Result<ResponseWrapper<TypeWrapper<CosmosAccountInfo>>, JsonRpcError> {
        let none: Option<bool> = None;
        self.jsonrpc_client
            .request_method(
                &format!("auth/accounts/{}", address),
                none,
                self.timeout,
                None,
            )
            .await
    }

    /// The advanced version of create_and_send transaction that expects you to
    /// perform your own signing and prep first.
    pub async fn send_transaction(&self, msg: Transaction) -> Result<TXSendResponse, JsonRpcError> {
        self.jsonrpc_client
            .request_method("txs", Some(msg), self.timeout, None)
            .await
    }

    /// The hand holding version of send transaction that does it all for you
    #[allow(clippy::too_many_arguments)]
    pub async fn create_and_send_transaction(
        &self,
        coin: Coin,
        fee: Coin,
        destination: Address,
        private_key: PrivateKey,
        chain_id: Option<String>,
        account_number: Option<u128>,
        sequence: Option<u128>,
    ) -> Result<TXSendResponse, JsonRpcError> {
        trace!("Creating transaction");
        let our_address = private_key
            .to_public_key()
            .expect("Invalid private key!")
            .to_address();

        let tx_info =
            maybe_get_optional_tx_info(our_address, chain_id, account_number, sequence, &self)
                .await?;

        let std_sign_msg = StdSignMsg {
            chain_id: tx_info.chain_id,
            account_number: tx_info.account_number,
            sequence: tx_info.sequence,
            fee: StdFee {
                amount: vec![fee],
                gas: 500_000u64.into(),
            },
            msgs: vec![Msg::SendMsg(SendMsg {
                from_address: our_address,
                to_address: destination,
                amount: vec![coin],
            })],
            memo: String::new(),
        };

        let tx = private_key
            .sign_std_msg(std_sign_msg, TransactionSendType::Block)
            .unwrap();
        trace!("{}", json!(tx));

        self.jsonrpc_client
            .request_method("txs", Some(tx), self.timeout, None)
            .await
    }

    /// Get the latest valset recorded by the peggy module. If no valset has ever been created
    /// you will instead get a blank valset at height 0. Any value above this may or may not
    /// be a complete valset and it's up to the caller to interpret the response.
    pub async fn get_peggy_valset(&self) -> Result<ResponseWrapper<Valset>, JsonRpcError> {
        let none: Option<bool> = None;
        let ret: Result<ResponseWrapper<ValsetUnparsed>, JsonRpcError> = self
            .jsonrpc_client
            .request_method("peggy/current_valset", none, self.timeout, None)
            .await;
        match ret {
            Ok(val) => Ok(ResponseWrapper {
                height: val.height,
                result: val.result.convert(),
            }),
            Err(e) => Err(e),
        }
    }

    /// get the valset for a given nonce (block) height
    pub async fn get_peggy_valset_request(
        &self,
        nonce: u128,
    ) -> Result<ResponseWrapper<Valset>, JsonRpcError> {
        let none: Option<bool> = None;
        let ret: Result<ResponseWrapper<TypeWrapper<ValsetUnparsed>>, JsonRpcError> = self
            .jsonrpc_client
            .request_method(
                &format!("peggy/valset_request/{}", nonce),
                none,
                self.timeout,
                None,
            )
            .await;
        match ret {
            Ok(val) => Ok(ResponseWrapper {
                height: val.height,
                result: val.result.value.convert(),
            }),
            Err(e) => Err(e),
        }
    }

    /// Send a transaction updating the eth address for the sending
    /// Cosmos address. The sending Cosmos address should be a validator
    pub async fn update_peggy_eth_address(
        &self,
        eth_private_key: EthPrivateKey,
        private_key: PrivateKey,
        fee: Coin,
        chain_id: Option<String>,
        account_number: Option<u128>,
        sequence: Option<u128>,
    ) -> Result<TXSendResponse, JsonRpcError> {
        trace!("Updating Peggy ETH address");
        let our_address = private_key
            .to_public_key()
            .expect("Invalid private key!")
            .to_address();

        let tx_info =
            maybe_get_optional_tx_info(our_address, chain_id, account_number, sequence, &self)
                .await?;
        trace!("got optional tx info");

        let eth_address = eth_private_key.to_public_key().unwrap();
        let eth_signature = eth_private_key.sign_msg(our_address.as_bytes());
        trace!(
            "sig: {} address: {}",
            clarity::utils::bytes_to_hex_str(&eth_signature.to_bytes()),
            clarity::utils::bytes_to_hex_str(eth_address.as_bytes())
        );

        let std_sign_msg = StdSignMsg {
            chain_id: tx_info.chain_id,
            account_number: tx_info.account_number,
            sequence: tx_info.sequence,
            fee: StdFee {
                amount: vec![fee],
                gas: 500_000u64.into(),
            },
            msgs: vec![Msg::SetEthAddressMsg(SetEthAddressMsg {
                eth_address,
                validator: our_address,
                eth_signature: bytes_to_hex_str(&eth_signature.to_bytes()),
            })],
            memo: String::new(),
        };

        let tx = private_key
            .sign_std_msg(std_sign_msg, TransactionSendType::Block)
            .unwrap();

        self.jsonrpc_client
            .request_method("txs", Some(tx), self.timeout, None)
            .await
    }

    /// Send a transaction requesting that a valset be formed for a given block
    /// height
    pub async fn send_valset_request(
        &self,
        private_key: PrivateKey,
        fee: Coin,
        chain_id: Option<String>,
        account_number: Option<u128>,
        sequence: Option<u128>,
    ) -> Result<TXSendResponse, JsonRpcError> {
        let our_address = private_key
            .to_public_key()
            .expect("Invalid private key!")
            .to_address();

        let tx_info =
            maybe_get_optional_tx_info(our_address, chain_id, account_number, sequence, &self)
                .await?;

        let std_sign_msg = StdSignMsg {
            chain_id: tx_info.chain_id,
            account_number: tx_info.account_number,
            sequence: tx_info.sequence,
            fee: StdFee {
                amount: vec![fee],
                gas: 500_000u64.into(),
            },
            msgs: vec![Msg::ValsetRequestMsg(ValsetRequestMsg {
                requester: our_address,
            })],
            memo: String::new(),
        };

        let tx = private_key
            .sign_std_msg(std_sign_msg, TransactionSendType::Block)
            .unwrap();
        trace!("{}", json!(tx));

        self.jsonrpc_client
            .request_method("txs", Some(tx), self.timeout, None)
            .await
    }

    /// Send in a confirmation for a specific validator set for a specific block height
    #[allow(clippy::too_many_arguments)]
    pub async fn send_valset_confirm(
        &self,
        eth_private_key: EthPrivateKey,
        fee: Coin,
        valset: Valset,
        private_key: PrivateKey,
        peggy_id: String,
        chain_id: Option<String>,
        account_number: Option<u128>,
        sequence: Option<u128>,
    ) -> Result<TXSendResponse, JsonRpcError> {
        let our_address = private_key
            .to_public_key()
            .expect("Invalid private key!")
            .to_address();

        let tx_info =
            maybe_get_optional_tx_info(our_address, chain_id, account_number, sequence, &self)
                .await?;

        let message = encode_tokens(&[
            Token::FixedString(peggy_id),
            Token::FixedString("checkpoint".to_string()),
            valset.nonce.into(),
            normalize_addresses(&valset.eth_addresses).into(),
            valset.powers.into(),
        ]);
        let eth_signature = eth_private_key.sign_msg(&message);

        let std_sign_msg = StdSignMsg {
            chain_id: tx_info.chain_id,
            account_number: tx_info.account_number,
            sequence: tx_info.sequence,
            fee: StdFee {
                amount: vec![fee],
                gas: 500_000u64.into(),
            },
            msgs: vec![Msg::ValsetConfirmMsg(ValsetConfirmMsg {
                validator: our_address,
                nonce: valset.nonce.into(),
                eth_signature: bytes_to_hex_str(&eth_signature.to_bytes()),
            })],
            memo: String::new(),
        };

        let tx = private_key
            .sign_std_msg(std_sign_msg, TransactionSendType::Block)
            .unwrap();

        self.jsonrpc_client
            .request_method("txs", Some(tx), self.timeout, None)
            .await
    }

    /// This hits the /pending_valset_requests endpoint and will provide the oldest
    /// validator set we have not yet signed.
    pub async fn oldest_unsigned_valset(
        &self,
        address: Address,
    ) -> Result<ResponseWrapper<Valset>, JsonRpcError> {
        let none: Option<bool> = None;
        let ret: Result<ResponseWrapper<ValsetUnparsed>, JsonRpcError> = self
            .jsonrpc_client
            .request_method(
                &format!("peggy/pending_valset_requests/{}", address),
                none,
                self.timeout,
                None,
            )
            .await;
        match ret {
            Ok(val) => Ok(ResponseWrapper {
                height: val.height,
                result: val.result.convert(),
            }),
            Err(e) => Err(e),
        }
    }

    /// this input views the last five valest requests that have been made, useful if you're
    /// a relayer looking to ferry confirmations
    pub async fn last_valset_requests(&self) -> Result<ResponseWrapper<Vec<Valset>>, JsonRpcError> {
        let none: Option<bool> = None;
        let ret: Result<ResponseWrapper<Vec<ValsetUnparsed>>, JsonRpcError> = self
            .jsonrpc_client
            .request_method(
                &"peggy/valset_requests".to_string(),
                none,
                self.timeout,
                None,
            )
            .await;

        match ret {
            Ok(val) => {
                let mut converted_values = Vec::new();
                for item in val.result {
                    converted_values.push(item.convert());
                }
                Ok(ResponseWrapper {
                    height: val.height,
                    result: converted_values,
                })
            }
            Err(e) => Err(e),
        }
    }

    /// get all valset confirmations for a given nonce
    pub async fn get_all_valset_confirms(
        &self,
        nonce: u64,
    ) -> Result<ValsetConfirmResponse, JsonRpcError> {
        let none: Option<bool> = None;
        let ret: Result<ValsetConfirmResponse, JsonRpcError> = self
            .jsonrpc_client
            .request_method(
                &format!("peggy/valset_confirm/{}", nonce),
                none,
                self.timeout,
                None,
            )
            .await;
        match ret {
            Ok(val) => Ok(val),
            Err(e) => Err(e),
        }
    }
}

/// Takes an array of Option<EthAddress> and converts to EthAddress by replacing None
/// values with a zero address
fn normalize_addresses(input: &[Option<EthAddress>]) -> Vec<EthAddress> {
    let mut output = Vec::new();
    for val in input.iter() {
        match val {
            Some(a) => output.push(*a),
            None => output.push(EthAddress::from_slice(&[0; 20]).unwrap()),
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix::Arbiter;
    use actix::System;
    use rand::{self, Rng};
    /// simple test used to get raw signature bytes to feed into other
    /// applications for testing. Specifically to get signing compatibility
    /// with go-ethereum
    #[test]
    #[ignore]
    fn get_sig() {
        use sha3::{Digest, Keccak256};
        let mut rng = rand::thread_rng();
        let secret: [u8; 32] = rng.gen();
        let eth_private_key = EthPrivateKey::from_slice(&secret).expect("Failed to parse eth key");
        let eth_address = eth_private_key.to_public_key().unwrap();
        let msg = eth_address.as_bytes();
        let eth_signature = eth_private_key.sign_msg(msg);
        let digest = Keccak256::digest(msg);
        println!(
            "sig: 0x{} hash: 0x{} address: 0x{}",
            clarity::utils::bytes_to_hex_str(&eth_signature.to_bytes()),
            clarity::utils::bytes_to_hex_str(&digest),
            clarity::utils::bytes_to_hex_str(eth_address.as_bytes())
        );
    }

    /// If you run the start-chains.sh script in the peggy repo it will pass
    /// port 1317 on localhost through to the peggycli rest-server which can
    /// then be used to run this test and debug things quickly. You will need
    /// to run the following command and copy a phrase so that you actually
    /// have some coins to send funds
    /// docker exec -it peggy_test_instance cat /validator-phrases
    #[test]
    #[ignore]
    fn test_endpoints() {
        let mut rng = rand::thread_rng();
        let secret: [u8; 32] = rng.gen();

        let key = PrivateKey::from_phrase("dog infant run timber spin chair owner craft wet insane tortoise hover labor letter doll funny mail piece arch depth unveil goddess flock crazy", "").unwrap();
        let eth_private_key = EthPrivateKey::from_slice(&secret).expect("Failed to parse eth key");
        let contact = Contact::new("http://localhost:1317", Duration::from_secs(30));
        let token_name = "footoken".to_string();

        let res = System::run(move || {
            Arbiter::spawn(async move {
                let res = test_rpc_calls(contact, key, eth_private_key, token_name).await;
                if res.is_err() {
                    println!("{:?}", res);
                    System::current().stop_with_code(1);
                }

                System::current().stop();
            });
        });

        if let Err(e) = res {
            panic!(format!("{:?}", e))
        }
    }
}

pub async fn test_rpc_calls(
    contact: Contact,
    key: PrivateKey,
    eth_private_key: EthPrivateKey,
    test_token_name: String,
) -> Result<(), String> {
    let fee = Coin {
        denom: test_token_name.clone(),
        amount: 1u32.into(),
    };
    let address = key
        .to_public_key()
        .expect("Failed to convert to pubkey!")
        .to_address();

    test_basic_calls(&contact, key, test_token_name, fee.clone(), address).await?;
    // set eth address also tested here, TODO expand to include things like changing
    // the set eth address
    test_valset_request_calls(&contact, key, eth_private_key, fee.clone()).await?;
    //test_valset_confirm_calls(&contact, key, eth_private_key, fee.clone()).await?;

    Ok(())
}

async fn test_basic_calls(
    contact: &Contact,
    key: PrivateKey,
    test_token_name: String,
    fee: Coin,
    address: Address,
) -> Result<(), String> {
    // start by validating the basics
    //
    // get the latest block
    // get our account info
    // send a base transaction

    let res = contact.get_latest_block().await;
    if res.is_err() {
        return Err(format!("Failed to get latest block {:?}", res));
    }

    let res = contact.get_account_info(address).await;
    if res.is_err() {
        return Err(format!("Failed to get account info {:?}", res));
    }

    let res = contact
        .create_and_send_transaction(
            Coin {
                denom: test_token_name.clone(),
                amount: 5u32.into(),
            },
            fee.clone(),
            key.to_public_key().unwrap().to_address(),
            key,
            None,
            None,
            None,
        )
        .await;
    if res.is_err() {
        return Err(format!("Failed to send tx {:?}", res));
    }
    Ok(())
}

async fn test_valset_request_calls(
    contact: &Contact,
    key: PrivateKey,
    eth_private_key: EthPrivateKey,
    fee: Coin,
) -> Result<(), String> {
    // next we update our eth address so that we can be sure it's present in the resulting valset
    // request
    let res = contact
        .update_peggy_eth_address(eth_private_key, key, fee.clone(), None, None, None)
        .await;
    if res.is_err() {
        return Err(format!("Failed to update eth address {:?}", res));
    }

    let res = contact.get_peggy_valset_request(1).await;
    if res.is_ok() {
        return Err(format!(
            "Got valset request that should not exist {:?}",
            res
        ));
    }

    // we request a valset be created
    // and then look at results at two block heights, one where the request was made, one where it
    // was not
    let res = contact
        .send_valset_request(key, fee.clone(), None, None, None)
        .await;
    if res.is_err() {
        return Err(format!("Failed to create valset request {:?}", res));
    }
    let valset_request_block = res.unwrap().height;

    let res = contact.get_peggy_valset_request(valset_request_block).await;
    println!("valset response is {:?}", res);
    if let Ok(valset) = res {
        assert_eq!(valset.height, valset_request_block);

        let addresses = valset.result.eth_addresses;
        if !addresses.contains(&Some(eth_private_key.to_public_key().unwrap())) {
            // we successfully submitted our eth address before, we should find it now
            return Err("Incorrect Valset, does not include submitted eth address".to_string());
        }
    } else {
        return Err("Failed to get valset request that should exist".to_string());
    }
    let res = contact.get_peggy_valset_request(valset_request_block).await;
    println!("valset response is {:?}", res);
    if let Ok(valset) = res {
        // this is actually a timing issue, but should be true
        assert_eq!(valset.height, valset_request_block);

        let addresses = valset.result.eth_addresses.clone();
        if !addresses.contains(&Some(eth_private_key.to_public_key().unwrap())) {
            // we successfully submitted our eth address before, we should find it now
            return Err("Incorrect Valset, does not include submitted eth address".to_string());
        }

        println!("Sending valset confirm!");
        // issue here, we can't actually test valset confirm because all the validators need
        // to have submitted an Ethereum address first.
        let res = contact
            .send_valset_confirm(
                eth_private_key,
                fee,
                valset.result,
                key,
                "test".to_string(),
                None,
                None,
                None,
            )
            .await;
        if res.is_err() {
            return Err(format!("Failed to send valset confirm {:?}", res));
        }
    } else {
        return Err("Failed to get valset request that should exist".to_string());
    }

    // valset confirm

    Ok(())
}
