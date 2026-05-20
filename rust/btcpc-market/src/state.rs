use crate::models::{Offer, Order, Product, ProductQA, ShippingAccount, Store};
use parking_lot::RwLock;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

#[derive(Default)]
pub struct MarketState {
    pub stores: HashMap<String, Store>,
    pub products: HashMap<String, Product>,
    pub orders: HashMap<String, Order>,
    pub reputation: HashMap<String, Vec<i64>>, // target_id → [weighted_sum, weight_total]
    pub offers: HashMap<String, Offer>,
    pub wishlists: HashMap<String, HashSet<String>>,        // username -> product_ids
    pub store_followers: HashMap<String, HashSet<String>>,  // seller -> follower usernames
    pub reputation_replies: HashMap<String, String>,         // vote_id -> reply text
    pub memo_keys: HashMap<String, String>,                  // username -> memo pubkey
}

pub type SharedState = Arc<RwLock<MarketState>>;

pub fn new_shared_state() -> SharedState {
    Arc::new(RwLock::new(MarketState::default()))
}

impl MarketState {
    pub fn apply_entry(&mut self, entry: &Value) {
        let entry_type = match entry.get("type").and_then(|v| v.as_str()) {
            Some(t) => t,
            None => return,
        };
        match entry_type {
            "STORE_OPEN"       => self.apply_store_open(entry),
            "STORE_UPDATE"     => self.apply_store_update(entry),
            "STORE_CLOSE"      => self.apply_store_close(entry),
            "PRODUCT_CREATE"   => self.apply_product_create(entry),
            "PRODUCT_UPDATE"   => self.apply_product_update(entry),
            "PRODUCT_DELIST"   => self.apply_product_delist(entry),
            "PRODUCT_BUMP"     => self.apply_product_bump(entry),
            "ORDER_PLACE"      => self.apply_order_place(entry),
            "ORDER_FULFILL"    => self.apply_order_fulfill(entry),
            "ORDER_DELIVERED"  => self.apply_order_delivered(entry),
            "ORDER_CANCEL"     => self.apply_order_cancel(entry),
            "ORDER_DISPUTE"    => self.apply_order_dispute(entry),
            "REPUTATION_VOTE"      => self.apply_reputation_vote(entry),
            "REPUTATION_REPLY"     => self.apply_reputation_reply(entry),
            "STORE_SHIPPING_LINK"  => self.apply_shipping_link(entry),
            "STORE_SHIPPING_UNLINK"=> self.apply_shipping_unlink(entry),
            "PRODUCT_QA_ASK"       => self.apply_qa_ask(entry),
            "PRODUCT_QA_ANSWER"    => self.apply_qa_answer(entry),
            "OFFER_PLACE"      => self.apply_offer_place(entry),
            "OFFER_COUNTER"    => self.apply_offer_counter(entry),
            "OFFER_ACCEPT"     => self.apply_offer_accept(entry),
            "OFFER_REJECT"     => self.apply_offer_reject(entry),
            "WISHLIST_ADD"     => self.apply_wishlist_add(entry),
            "WISHLIST_REMOVE"  => self.apply_wishlist_remove(entry),
            "FOLLOW_STORE"     => self.apply_follow_store(entry),
            "UNFOLLOW_STORE"   => self.apply_unfollow_store(entry),
            "MEMO_KEY_REGISTER" => self.apply_memo_key_register(entry),
            _ => {}
        }
    }

    fn apply_store_open(&mut self, e: &Value) {
        let seller = s(e, "from");
        let sd = &e["store_data"];
        self.stores.insert(seller.clone(), Store {
            seller: seller.clone(),
            name: sd["name"].as_str().unwrap_or(&seller).to_string(),
            banner_cid: sd["banner_cid"].as_str().map(str::to_string),
            description_cid: sd["description_cid"].as_str().map(str::to_string),
            categories: arr_strings(&sd["categories"]),
            capacity: sd["capacity"].as_u64().unwrap_or(0) as u32,
            used_capacity: 0,
            stake_amount: sd["stake_amount"].as_f64().unwrap_or(0.0),
            status: "active".to_string(),
            opened_at: e["timestamp"].as_u64().unwrap_or(0),
            score: 0.0,
            shipping_accounts: vec![],
            tor_enabled: false,
            onion_address: None,
        });
    }

    fn apply_store_update(&mut self, e: &Value) {
        let seller = s(e, "from");
        let sd = &e["store_data"];
        if let Some(store) = self.stores.get_mut(&seller) {
            if let Some(name) = sd["name"].as_str() { store.name = name.to_string(); }
            if sd["banner_cid"].is_string() {
                store.banner_cid = sd["banner_cid"].as_str().map(str::to_string);
            }
            if sd["description_cid"].is_string() {
                store.description_cid = sd["description_cid"].as_str().map(str::to_string);
            }
            if sd["categories"].is_array() {
                store.categories = arr_strings(&sd["categories"]);
            }
            if let Some(onion) = sd["onion_address"].as_str() {
                store.onion_address = Some(onion.to_string());
                store.tor_enabled = true;
            }
            if sd["tor_enabled"] == serde_json::Value::Bool(false) {
                store.tor_enabled = false;
            }
        }
    }

    fn apply_store_close(&mut self, e: &Value) {
        let seller = s(e, "from");
        if let Some(store) = self.stores.get_mut(&seller) {
            store.status = "closed".to_string();
        }
    }

    fn apply_product_create(&mut self, e: &Value) {
        let seller = s(e, "from");
        let pd = &e["product_data"];
        let product_id = pd["product_id"].as_str().unwrap_or("").to_string();
        if product_id.is_empty() { return; }
        if let Some(store) = self.stores.get_mut(&seller) {
            store.used_capacity = store.used_capacity.saturating_add(1);
        }
        self.products.insert(product_id.clone(), Product {
            product_id,
            seller,
            title: pd["title"].as_str().unwrap_or("").to_string(),
            description: pd["description"].as_str().map(str::to_string),
            price: pd["price"].as_f64().unwrap_or(0.0),
            token: pd["token"].as_str().unwrap_or("BTCPC").to_string(),
            image_cid: pd["image_cid"].as_str().map(str::to_string),
            inventory: pd["inventory"].as_u64().map(|n| n as u32),
            categories: arr_strings(&pd["categories"]),
            status: "active".to_string(),
            created_epoch: e["epoch"].as_u64().unwrap_or(0),
            auto_deliver: pd["auto_deliver"].as_bool().unwrap_or(false),
            delivery_cid: pd["delivery_cid"].as_str().map(str::to_string),
            sale_price: pd["sale_price"].as_f64(),
            sale_ends_epoch: pd["sale_ends_epoch"].as_u64(),
            questions: vec![],
            min_price: pd["min_price"].as_f64(),
            suggested_price: pd["suggested_price"].as_f64(),
            tip_enabled: pd["tip_enabled"].as_bool().unwrap_or(false),
            condition: pd["condition"].as_str().map(str::to_string),
            condition_notes: pd["condition_notes"].as_str().map(str::to_string),
            expected_delivery_epoch: pd["expected_delivery_epoch"].as_u64(),
            specs: pd.get("specs").and_then(|v| if v.is_null() { None } else { Some(v.clone()) }),
            image_cids: arr_strings(&pd["image_cids"]),
            bumped_until_epoch: pd["bumped_until_epoch"].as_u64(),
        });
    }

    fn apply_product_update(&mut self, e: &Value) {
        let pd = &e["product_data"];
        let product_id = pd["product_id"].as_str().unwrap_or("").to_string();
        if let Some(product) = self.products.get_mut(&product_id) {
            if let Some(title) = pd["title"].as_str() { product.title = title.to_string(); }
            if let Some(desc) = pd["description"].as_str() {
                product.description = Some(desc.to_string());
            }
            if let Some(price) = pd["price"].as_f64() { product.price = price; }
            if let Some(cid) = pd["image_cid"].as_str() {
                product.image_cid = Some(cid.to_string());
            }
            if let Some(inv) = pd["inventory"].as_i64() {
                product.inventory = if inv < 0 { None } else { Some(inv as u32) };
            }
            if let Some(ad) = pd["auto_deliver"].as_bool() { product.auto_deliver = ad; }
            if pd["delivery_cid"].is_string() {
                product.delivery_cid = pd["delivery_cid"].as_str().map(str::to_string);
            }
            if let Some(sp) = pd["sale_price"].as_f64() { product.sale_price = Some(sp); }
            if let Some(se) = pd["sale_ends_epoch"].as_u64() { product.sale_ends_epoch = Some(se); }
            if let Some(mp) = pd["min_price"].as_f64() { product.min_price = Some(mp); }
            if let Some(sgp) = pd["suggested_price"].as_f64() { product.suggested_price = Some(sgp); }
            if let Some(te) = pd["tip_enabled"].as_bool() { product.tip_enabled = te; }
            if let Some(cond) = pd["condition"].as_str() { product.condition = Some(cond.to_string()); }
            if let Some(cn) = pd["condition_notes"].as_str() { product.condition_notes = Some(cn.to_string()); }
            if let Some(ede) = pd["expected_delivery_epoch"].as_u64() { product.expected_delivery_epoch = Some(ede); }
            if let Some(specs) = pd.get("specs") {
                if !specs.is_null() { product.specs = Some(specs.clone()); }
            }
            if pd["image_cids"].is_array() { product.image_cids = arr_strings(&pd["image_cids"]); }
            if let Some(bue) = pd["bumped_until_epoch"].as_u64() { product.bumped_until_epoch = Some(bue); }
        }
    }

    fn apply_product_delist(&mut self, e: &Value) {
        let pd = &e["product_data"];
        let product_id = pd["product_id"].as_str().unwrap_or("").to_string();
        // Read seller before mutating product (avoids double mutable borrow)
        let seller = self.products.get(&product_id).map(|p| p.seller.clone());
        if let Some(product) = self.products.get_mut(&product_id) {
            product.status = "delisted".to_string();
        }
        if let Some(seller) = seller {
            if let Some(store) = self.stores.get_mut(&seller) {
                store.used_capacity = store.used_capacity.saturating_sub(1);
            }
        }
    }

    fn apply_order_place(&mut self, e: &Value) {
        let od = &e["order_data"];
        let order_id = od["order_id"].as_str().unwrap_or("").to_string();
        if order_id.is_empty() { return; }
        let quantity = od["quantity"].as_u64().unwrap_or(1) as u32;
        if let Some(pid) = od["product_id"].as_str() {
            if let Some(product) = self.products.get_mut(pid) {
                if let Some(inv) = product.inventory {
                    product.inventory = Some(inv.saturating_sub(quantity));
                }
            }
        }
        let placed_epoch = e["epoch"].as_u64().unwrap_or(0);
        self.orders.insert(order_id.clone(), Order {
            order_id,
            buyer: s(e, "from"),
            seller: s(e, "to"),
            product_id: od["product_id"].as_str().unwrap_or("").to_string(),
            quantity,
            unit_price: od["unit_price"].as_f64().unwrap_or(0.0),
            total: od["total"].as_f64().unwrap_or(0.0),
            token: od["token"].as_str().unwrap_or("BTCPC").to_string(),
            escrow_id: od["escrow_id"].as_str().map(str::to_string),
            escrow_locked: true,
            escrow_amount: od["total"].as_f64().unwrap_or(0.0),
            status: "pending".to_string(),
            fulfillment_cid: None,
            placed_epoch,
            fulfill_deadline_epoch: placed_epoch + 4800,
            shipping_address: od["shipping_address"].as_str().map(str::to_string),
            carrier: None,
            tracking_number: None,
            shipping_service: None,
            shipping_note: None,
            tip_amount: od["tip_amount"].as_f64(),
            license_key: None,
            fulfilled_epoch: None,
        });
    }

    fn apply_order_fulfill(&mut self, e: &Value) {
        let od = &e["order_data"];
        if let Some(oid) = od["order_id"].as_str() {
            if let Some(order) = self.orders.get_mut(oid) {
                order.status = "fulfilled".to_string();
                order.fulfilled_epoch = Some(e["epoch"].as_u64().unwrap_or(0));
                order.fulfillment_cid = od["fulfillment_cid"].as_str().map(str::to_string);
                order.carrier          = od["carrier"].as_str().map(str::to_string);
                order.tracking_number  = od["tracking_number"].as_str().map(str::to_string);
                order.shipping_service = od["shipping_service"].as_str().map(str::to_string);
                order.shipping_note    = od["shipping_note"].as_str().map(str::to_string);
                if let Some(lk) = od["license_key"].as_str() {
                    order.license_key = Some(lk.to_string());
                }
            }
        }
    }

    fn apply_order_delivered(&mut self, e: &Value) {
        let od = &e["order_data"];
        if let Some(oid) = od["order_id"].as_str() {
            if let Some(order) = self.orders.get_mut(oid) {
                order.status = "delivered".to_string();
                order.escrow_locked = false;
            }
        }
    }

    fn apply_order_cancel(&mut self, e: &Value) {
        let od = &e["order_data"];
        if let Some(oid) = od["order_id"].as_str() {
            // Read product_id + quantity before mutably borrowing orders
            let restore = self.orders.get(oid)
                .map(|o| (o.product_id.clone(), o.quantity));
            if let Some(order) = self.orders.get_mut(oid) {
                order.status = "cancelled".to_string();
                order.escrow_locked = false;
            }
            if let Some((pid, qty)) = restore {
                if let Some(product) = self.products.get_mut(&pid) {
                    if let Some(inv) = product.inventory {
                        product.inventory = Some(inv + qty);
                    }
                }
            }
        }
    }

    fn apply_order_dispute(&mut self, e: &Value) {
        let od = &e["order_data"];
        if let Some(oid) = od["order_id"].as_str() {
            if let Some(order) = self.orders.get_mut(oid) {
                order.status = "disputed".to_string();
            }
        }
    }

    fn apply_shipping_link(&mut self, e: &Value) {
        let seller = s(e, "from");
        let sd = &e["store_data"];
        let carrier    = sd["carrier"].as_str().unwrap_or("").to_string();
        let account_id = sd["account_id"].as_str().unwrap_or("").to_string();
        if carrier.is_empty() || account_id.is_empty() { return; }
        let masked = if account_id.len() <= 4 {
            "*".repeat(account_id.len())
        } else {
            format!("****{}", &account_id[account_id.len() - 4..])
        };
        let default_service = sd["default_service"].as_str().unwrap_or("ground").to_string();
        let linked_at = e["timestamp"].as_u64().unwrap_or(0);
        if let Some(store) = self.stores.get_mut(&seller) {
            if let Some(existing) = store.shipping_accounts.iter_mut().find(|a| a.carrier == carrier) {
                existing.account_id_masked = masked;
                existing.default_service   = default_service;
                existing.linked_at         = linked_at;
            } else {
                store.shipping_accounts.push(ShippingAccount { carrier, account_id_masked: masked, default_service, linked_at });
            }
        }
    }

    fn apply_shipping_unlink(&mut self, e: &Value) {
        let seller  = s(e, "from");
        let carrier = e["store_data"]["carrier"].as_str().unwrap_or("").to_string();
        if let Some(store) = self.stores.get_mut(&seller) {
            store.shipping_accounts.retain(|a| a.carrier != carrier);
        }
    }

    fn apply_qa_ask(&mut self, e: &Value) {
        let pd = &e["product_data"];
        let product_id = pd["product_id"].as_str().unwrap_or("").to_string();
        let qa_id = pd["qa_id"].as_str().unwrap_or("").to_string();
        if product_id.is_empty() || qa_id.is_empty() { return; }
        if let Some(product) = self.products.get_mut(&product_id) {
            product.questions.push(ProductQA {
                qa_id,
                asker: s(e, "from"),
                question: pd["question"].as_str().unwrap_or("").to_string(),
                answer: None,
                asked_epoch: e["epoch"].as_u64().unwrap_or(0),
                answered_epoch: None,
            });
        }
    }

    fn apply_qa_answer(&mut self, e: &Value) {
        let pd = &e["product_data"];
        let product_id = pd["product_id"].as_str().unwrap_or("").to_string();
        let qa_id = pd["qa_id"].as_str().unwrap_or("").to_string();
        if let Some(product) = self.products.get_mut(&product_id) {
            if let Some(qa) = product.questions.iter_mut().find(|q| q.qa_id == qa_id) {
                qa.answer = pd["answer"].as_str().map(str::to_string);
                qa.answered_epoch = Some(e["epoch"].as_u64().unwrap_or(0));
            }
        }
    }

    fn apply_product_bump(&mut self, e: &Value) {
        let pd = &e["product_data"];
        let product_id = pd["product_id"].as_str().unwrap_or("").to_string();
        if let Some(product) = self.products.get_mut(&product_id) {
            if let Some(bue) = pd["bumped_until_epoch"].as_u64() {
                product.bumped_until_epoch = Some(bue);
            }
        }
    }

    fn apply_offer_place(&mut self, e: &Value) {
        let od = &e["order_data"];
        let offer_id = od["offer_id"].as_str().unwrap_or("").to_string();
        if offer_id.is_empty() { return; }
        let product_id = od["product_id"].as_str().unwrap_or("").to_string();
        let buyer = s(e, "from");
        let seller = self.products.get(&product_id).map(|p| p.seller.clone()).unwrap_or_default();
        let created_epoch = e["epoch"].as_u64().unwrap_or(0);
        let expires_epoch = od["expires_epoch"].as_u64().unwrap_or(created_epoch + 2880);
        self.offers.insert(offer_id.clone(), crate::models::Offer {
            offer_id,
            product_id,
            buyer,
            seller,
            amount: od["amount"].as_f64().unwrap_or(0.0),
            counter_amount: None,
            message: od["message"].as_str().map(str::to_string),
            counter_message: None,
            status: "pending".to_string(),
            created_epoch,
            expires_epoch,
            order_id: None,
        });
    }

    fn apply_offer_counter(&mut self, e: &Value) {
        let od = &e["order_data"];
        if let Some(offer_id) = od["offer_id"].as_str() {
            if let Some(offer) = self.offers.get_mut(offer_id) {
                offer.status = "countered".to_string();
                offer.counter_amount = od["counter_amount"].as_f64();
                offer.counter_message = od["counter_message"].as_str().map(str::to_string);
            }
        }
    }

    fn apply_offer_accept(&mut self, e: &Value) {
        let od = &e["order_data"];
        if let Some(offer_id) = od["offer_id"].as_str() {
            if let Some(offer) = self.offers.get_mut(offer_id) {
                offer.status = "accepted".to_string();
                if let Some(oid) = od["order_id"].as_str() {
                    offer.order_id = Some(oid.to_string());
                }
            }
        }
    }

    fn apply_offer_reject(&mut self, e: &Value) {
        let od = &e["order_data"];
        if let Some(offer_id) = od["offer_id"].as_str() {
            if let Some(offer) = self.offers.get_mut(offer_id) {
                offer.status = "rejected".to_string();
            }
        }
    }

    fn apply_wishlist_add(&mut self, e: &Value) {
        let buyer = s(e, "from");
        let pd = &e["product_data"];
        if let Some(pid) = pd["product_id"].as_str() {
            self.wishlists.entry(buyer).or_default().insert(pid.to_string());
        }
    }

    fn apply_wishlist_remove(&mut self, e: &Value) {
        let buyer = s(e, "from");
        let pd = &e["product_data"];
        if let Some(pid) = pd["product_id"].as_str() {
            if let Some(set) = self.wishlists.get_mut(&buyer) {
                set.remove(pid);
            }
        }
    }

    fn apply_follow_store(&mut self, e: &Value) {
        let follower = s(e, "from");
        let sd = &e["store_data"];
        if let Some(seller) = sd["seller"].as_str() {
            self.store_followers.entry(seller.to_string()).or_default().insert(follower);
        }
    }

    fn apply_unfollow_store(&mut self, e: &Value) {
        let follower = s(e, "from");
        let sd = &e["store_data"];
        if let Some(seller) = sd["seller"].as_str() {
            if let Some(set) = self.store_followers.get_mut(seller) {
                set.remove(&follower);
            }
        }
    }

    fn apply_reputation_reply(&mut self, e: &Value) {
        let rd = &e["reputation_data"];
        if let Some(vote_id) = rd["vote_id"].as_str() {
            if let Some(reply) = rd["reply"].as_str() {
                self.reputation_replies.insert(vote_id.to_string(), reply.to_string());
            }
        }
    }

    fn apply_reputation_vote(&mut self, e: &Value) {
        let rd = &e["reputation_data"];
        let target_id = rd["target_id"].as_str().unwrap_or("").to_string();
        if target_id.is_empty() { return; }
        let target_type = rd["target_type"].as_str().unwrap_or("").to_string();
        let vote = rd["vote"].as_i64().unwrap_or(0);
        let weight = rd["weight"].as_i64().unwrap_or(1);

        // Update reputation map, capturing results before releasing borrow
        let (sum, count) = {
            let rep = self.reputation.entry(target_id.clone()).or_insert_with(|| vec![0, 0]);
            rep[0] += vote * weight;
            rep[1] += weight;
            (rep[0], rep[1])
        };

        // Now we can safely borrow stores
        if target_type == "store" && count > 0 {
            if let Some(store) = self.stores.get_mut(&target_id) {
                store.score = (sum as f64 / count as f64 * 5.0).clamp(0.0, 5.0);
            }
        }
    }

    fn apply_memo_key_register(&mut self, e: &Value) {
        let actor = s(e, "from");
        if let Some(pubkey) = e["memo_key_data"]["memo_pubkey"].as_str() {
            if !pubkey.is_empty() {
                self.memo_keys.insert(actor, pubkey.to_string());
            }
        }
    }
}

fn s(e: &Value, key: &str) -> String {
    e[key].as_str().unwrap_or("").to_string()
}

fn arr_strings(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}
