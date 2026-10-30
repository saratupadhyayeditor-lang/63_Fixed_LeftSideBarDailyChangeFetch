//! The DhanHQ REST client.
//!
//! One [`DhanClient`] wraps the `access-token`/`client-id` credentials and
//! exposes the documented v2 endpoints. Every call returns either the typed
//! response or a [`DhanError`] that preserves Dhan's `DH-9xx` code.

use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::error::{DhanError, Result};
use crate::models::*;

pub const DEFAULT_BASE: &str = "https://api.dhan.co/v2";

/// A connected (or connectable) DhanHQ account.
#[derive(Clone)]
pub struct DhanClient {
    http: reqwest::Client,
    client_id: String,
    access_token: String,
    base: String,
}

impl DhanClient {
    /// Create a client. No network call is made; call [`DhanClient::profile`] to
    /// validate the credentials.
    pub fn new(client_id: impl Into<String>, access_token: impl Into<String>) -> Self {
        // Dhan's default REST endpoints must answer fast; a hung pre-market call
        // should fail rather than block a request thread for 60s.
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .connect_timeout(std::time::Duration::from_secs(8))
            .build()
            .unwrap_or_default();
        Self {
            http,
            client_id: client_id.into(),
            access_token: access_token.into(),
            base: DEFAULT_BASE.to_string(),
        }
    }

    pub fn with_base(mut self, base: impl Into<String>) -> Self {
        self.base = base.into().trim_end_matches('/').to_string();
        self
    }

    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    /// The access token. Exposed so the feed layer can reuse the same
    /// credentials; callers must never log or persist it.
    pub fn access_token(&self) -> &str {
        &self.access_token
    }

    /// Send a request and decode the JSON body, mapping HTTP/JSON failures.
    async fn send<T: DeserializeOwned>(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&Value>,
        data_header: bool,
    ) -> Result<T> {
        let url = format!("{}{}", self.base, path);
        let mut req = self
            .http
            .request(method, &url)
            .header("Accept", "application/json")
            .header("access-token", &self.access_token);
        if data_header {
            req = req.header("client-id", &self.client_id);
        }
        if let Some(b) = body {
            req = req.header("Content-Type", "application/json").json(b);
        }
        let resp = req.send().await?;
        let status = resp.status();
        let text = resp.text().await?;
        if !status.is_success() {
            return Err(DhanError::from_body(status, &text));
        }
        // Some Dhan endpoints answer 200 with an error envelope.
        if let Ok(v) = serde_json::from_str::<Value>(&text) {
            if let Some(code) = v.get("errorCode").and_then(|c| c.as_str()) {
                if !code.is_empty() {
                    return Err(DhanError::Api {
                        code: code.to_string(),
                        error_type: v
                            .get("errorType")
                            .and_then(|c| c.as_str())
                            .unwrap_or("")
                            .to_string(),
                        message: v
                            .get("errorMessage")
                            .and_then(|c| c.as_str())
                            .unwrap_or("")
                            .to_string(),
                    });
                }
            }
        }
        serde_json::from_str::<T>(&text).map_err(|e| {
            DhanError::Invalid(format!("{} (body: {})", e, truncate(&text, 300)))
        })
    }

    // -- profile / account --------------------------------------------------

    /// `GET /profile` - validates the token and returns account metadata.
    pub async fn profile(&self) -> Result<Profile> {
        self.send(reqwest::Method::GET, "/profile", None, false)
            .await
    }

    /// `GET /fundlimit`
    pub async fn fund_limit(&self) -> Result<FundLimit> {
        self.send(reqwest::Method::GET, "/fundlimit", None, false)
            .await
    }

    /// `POST /margincalculator`
    pub async fn margin_calculator(&self, req: &MarginRequest) -> Result<MarginResponse> {
        let body = serde_json::to_value(req).map_err(|e| DhanError::Message(e.to_string()))?;
        self.send(reqwest::Method::POST, "/margincalculator", Some(&body), false)
            .await
    }

    // -- orders -------------------------------------------------------------

    /// `POST /orders`
    pub async fn place_order(&self, req: &OrderRequest) -> Result<OrderResponse> {
        let body = serde_json::to_value(req).map_err(|e| DhanError::Message(e.to_string()))?;
        self.send(reqwest::Method::POST, "/orders", Some(&body), false)
            .await
    }

    /// `POST /orders/slicing`
    pub async fn slice_order(&self, req: &OrderRequest) -> Result<Vec<OrderResponse>> {
        let body = serde_json::to_value(req).map_err(|e| DhanError::Message(e.to_string()))?;
        self.send(reqwest::Method::POST, "/orders/slicing", Some(&body), false)
            .await
    }

    /// `PUT /orders/{order-id}`
    pub async fn modify_order(&self, req: &ModifyOrderRequest) -> Result<OrderResponse> {
        let body = serde_json::to_value(req).map_err(|e| DhanError::Message(e.to_string()))?;
        self.send(
            reqwest::Method::PUT,
            &format!("/orders/{}", req.order_id),
            Some(&body),
            false,
        )
        .await
    }

    /// `DELETE /orders/{order-id}`
    pub async fn cancel_order(&self, order_id: &str) -> Result<OrderResponse> {
        self.send(
            reqwest::Method::DELETE,
            &format!("/orders/{}", order_id),
            None,
            false,
        )
        .await
    }

    /// `GET /orders`
    pub async fn order_book(&self) -> Result<Vec<Order>> {
        self.send(reqwest::Method::GET, "/orders", None, false).await
    }

    /// `GET /orders/{order-id}`
    pub async fn order_by_id(&self, order_id: &str) -> Result<Order> {
        self.send(
            reqwest::Method::GET,
            &format!("/orders/{}", order_id),
            None,
            false,
        )
        .await
    }

    /// `GET /orders/external/{correlation-id}`
    pub async fn order_by_correlation(&self, correlation_id: &str) -> Result<Order> {
        self.send(
            reqwest::Method::GET,
            &format!("/orders/external/{}", correlation_id),
            None,
            false,
        )
        .await
    }

    /// `POST /super/orders` - a Dhan Super Order (entry + target + stop-loss +
    /// optional trailing leg), all self-protecting on the broker side.
    pub async fn place_super_order(&self, body: &Value) -> Result<Value> {
        self.send(reqwest::Method::POST, "/super/orders", Some(body), false)
            .await
    }

    /// `GET /super/orders` - the live Super Order book with nested leg details
    /// (current stop-loss price and trailing jump).
    pub async fn super_orders(&self) -> Result<Vec<Value>> {
        self.send(reqwest::Method::GET, "/super/orders", None, false)
            .await
    }

    /// `PUT /super/orders/{order-id}`
    pub async fn modify_super_order(&self, order_id: &str, body: &Value) -> Result<Value> {
        self.send(
            reqwest::Method::PUT,
            &format!("/super/orders/{}", order_id),
            Some(body),
            false,
        )
        .await
    }

    /// `DELETE /super/orders/{order-id}`
    pub async fn cancel_super_order(&self, order_id: &str) -> Result<Value> {
        self.send(
            reqwest::Method::DELETE,
            &format!("/super/orders/{}", order_id),
            None,
            false,
        )
        .await
    }

    /// `POST /forever/orders` - a Good-Till-Triggered order (single or OCO).
    pub async fn place_forever(&self, body: &Value) -> Result<Value> {
        self.send(reqwest::Method::POST, "/forever/orders", Some(body), false)
            .await
    }

    /// `GET /forever/orders`
    pub async fn forever_orders(&self) -> Result<Vec<Value>> {
        self.send(reqwest::Method::GET, "/forever/orders", None, false)
            .await
    }

    /// `DELETE /forever/orders/{order-id}`
    pub async fn cancel_forever(&self, order_id: &str) -> Result<Value> {
        self.send(
            reqwest::Method::DELETE,
            &format!("/forever/orders/{}", order_id),
            None,
            false,
        )
        .await
    }

    /// `GET /trades`
    pub async fn trade_book(&self) -> Result<Vec<Trade>> {
        self.send(reqwest::Method::GET, "/trades", None, false).await
    }

    /// `GET /trades/{order-id}`
    pub async fn trades_for_order(&self, order_id: &str) -> Result<Vec<Trade>> {
        self.send(
            reqwest::Method::GET,
            &format!("/trades/{}", order_id),
            None,
            false,
        )
        .await
    }

    // -- portfolio ----------------------------------------------------------

    /// `GET /holdings`
    pub async fn holdings(&self) -> Result<Vec<Holding>> {
        self.send(reqwest::Method::GET, "/holdings", None, false)
            .await
    }

    /// `GET /positions`
    pub async fn positions(&self) -> Result<Vec<Position>> {
        self.send(reqwest::Method::GET, "/positions", None, false)
            .await
    }

    /// `POST /positions/convert`
    pub async fn convert_position(&self, req: &ConvertPositionRequest) -> Result<Value> {
        let body = serde_json::to_value(req).map_err(|e| DhanError::Message(e.to_string()))?;
        self.send(
            reqwest::Method::POST,
            "/positions/convert",
            Some(&body),
            false,
        )
        .await
    }

    /// `DELETE /positions`
    pub async fn exit_all_positions(&self) -> Result<Value> {
        self.send(reqwest::Method::DELETE, "/positions", None, false)
            .await
    }

    // -- market quote -------------------------------------------------------

    async fn marketfeed(&self, path: &str, req: &SegmentInstruments) -> Result<MarketFeedResponse> {
        let body = serde_json::to_value(req).map_err(|e| DhanError::Message(e.to_string()))?;
        let env: Envelope<MarketFeedResponse> = self
            .send(reqwest::Method::POST, path, Some(&body), true)
            .await?;
        // Dhan answers 200 with an empty/ovoid body when it rejects the feed
        // subscription (bad segment list, closed session, token scope). Surface
        // that instead of silently returning zero quotes.
        match env.data {
            Some(d) if !d.is_empty() => Ok(d),
            _ => Err(DhanError::Message(format!(
                "marketfeed {path} returned no data (status={:?}, auth_error={:?})",
                env.status, env.auth_error
            ))),
        }
    }

    /// `POST /marketfeed/ltp`
    pub async fn market_feed_ltp(&self, req: &SegmentInstruments) -> Result<MarketFeedResponse> {
        self.marketfeed("/marketfeed/ltp", req).await
    }

    /// `POST /marketfeed/ohlc`
    pub async fn market_feed_ohlc(&self, req: &SegmentInstruments) -> Result<MarketFeedResponse> {
        self.marketfeed("/marketfeed/ohlc", req).await
    }

    /// `POST /marketfeed/quote`
    pub async fn market_feed_quote(&self, req: &SegmentInstruments) -> Result<MarketFeedResponse> {
        self.marketfeed("/marketfeed/quote", req).await
    }

    // -- historical data ----------------------------------------------------

    /// `POST /charts/intraday`
    pub async fn intraday(&self, req: &IntradayRequest) -> Result<CandleData> {
        let body = serde_json::to_value(req).map_err(|e| DhanError::Message(e.to_string()))?;
        self.send(reqwest::Method::POST, "/charts/intraday", Some(&body), false)
            .await
    }

    /// `POST /charts/historical`
    pub async fn historical(&self, req: &HistoricalRequest) -> Result<CandleData> {
        let body = serde_json::to_value(req).map_err(|e| DhanError::Message(e.to_string()))?;
        self.send(reqwest::Method::POST, "/charts/historical", Some(&body), false)
            .await
    }

    // -- option chain -------------------------------------------------------

    /// `POST /optionchain/expirylist`
    pub async fn option_chain_expiry_list(&self, req: &OptionChainRequest) -> Result<Vec<String>> {
        let body = serde_json::to_value(req).map_err(|e| DhanError::Message(e.to_string()))?;
        let env: Envelope<Vec<String>> = self
            .send(
                reqwest::Method::POST,
                "/optionchain/expirylist",
                Some(&body),
                true,
            )
            .await?;
        Ok(env.data.unwrap_or_default())
    }

    /// `POST /optionchain`
    pub async fn option_chain(&self, req: &OptionChainRequest) -> Result<OptionChainData> {
        let body = serde_json::to_value(req).map_err(|e| DhanError::Message(e.to_string()))?;
        let resp: OptionChainResponse = self
            .send(reqwest::Method::POST, "/optionchain", Some(&body), true)
            .await?;
        Ok(resp.data)
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max])
    }
}
