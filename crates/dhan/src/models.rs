//! Typed request/response models for the DhanHQ v2 API.
//!
//! Field names and enum values mirror the official documentation exactly so a
//! payload round-trips without a mapping layer. Numeric ids are kept as strings
//! because Dhan's REST contract expects `"securityId":"11536"` style values.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Enums (Annexure)
// ---------------------------------------------------------------------------

macro_rules! string_enum {
    ($name:ident, $default:ident { $($variant:ident => $s:literal),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        pub enum $name {
            $(#[serde(rename = $s)] $variant),+
        }
        impl Default for $name {
            fn default() -> Self {
                Self::$default
            }
        }
        impl $name {
            pub fn as_str(&self) -> &'static str {
                match self { $(Self::$variant => $s),+ }
            }
        }
        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}

string_enum!(ExchangeSegment, NseEq {
    IdxI => "IDX_I",
    NseEq => "NSE_EQ",
    NseFno => "NSE_FNO",
    NseCurrency => "NSE_CURRENCY",
    BseEq => "BSE_EQ",
    McxComm => "MCX_COMM",
    BseCurrency => "BSE_CURRENCY",
    BseFno => "BSE_FNO",
});

string_enum!(TransactionType, Buy { Buy => "BUY", Sell => "SELL" });

string_enum!(ProductType, Intraday {
    Cnc => "CNC",
    Intraday => "INTRADAY",
    Margin => "MARGIN",
    Mtf => "MTF",
    Co => "CO",
    Bo => "BO",
});

string_enum!(OrderType, Market {
    Limit => "LIMIT",
    Market => "MARKET",
    StopLoss => "STOP_LOSS",
    StopLossMarket => "STOP_LOSS_MARKET",
});

string_enum!(Validity, Day { Day => "DAY", Ioc => "IOC" });

string_enum!(AmoTime, Open {
    PreOpen => "PRE_OPEN",
    Open => "OPEN",
    Open30 => "OPEN_30",
    Open60 => "OPEN_60",
});

string_enum!(PositionType, Long { Long => "LONG", Short => "SHORT", Closed => "CLOSED" });

string_enum!(LegName, EntryLeg {
    EntryLeg => "ENTRY_LEG",
    TargetLeg => "TARGET_LEG",
    StopLossLeg => "STOP_LOSS_LEG",
});

string_enum!(Instrument, Equity {
    Index => "INDEX",
    FutIdx => "FUTIDX",
    OptIdx => "OPTIDX",
    Equity => "EQUITY",
    FutStk => "FUTSTK",
    OptStk => "OPTSTK",
    FutCom => "FUTCOM",
    OptFut => "OPTFUT",
    FutCur => "FUTCUR",
    OptCur => "OPTCUR",
});

// ---------------------------------------------------------------------------
// Trading: orders
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct OrderRequest {
    pub dhan_client_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
    pub transaction_type: TransactionType,
    pub exchange_segment: ExchangeSegment,
    pub product_type: ProductType,
    pub order_type: OrderType,
    pub validity: Validity,
    pub security_id: String,
    pub quantity: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disclosed_quantity: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub price: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trigger_price: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after_market_order: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amo_time: Option<AmoTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bo_profit_value: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bo_stop_loss_value: Option<f64>,
}

impl OrderRequest {
    /// A market order with the fields Dhan requires for the common intraday case.
    pub fn market(
        client_id: impl Into<String>,
        security_id: impl Into<String>,
        segment: ExchangeSegment,
        side: TransactionType,
        quantity: i64,
    ) -> Self {
        Self {
            dhan_client_id: client_id.into(),
            transaction_type: side,
            exchange_segment: segment,
            product_type: ProductType::Intraday,
            order_type: OrderType::Market,
            validity: Validity::Day,
            security_id: security_id.into(),
            quantity,
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ModifyOrderRequest {
    pub dhan_client_id: String,
    pub order_id: String,
    pub order_type: OrderType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub leg_name: Option<LegName>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub price: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disclosed_quantity: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trigger_price: Option<f64>,
    pub validity: Validity,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct OrderResponse {
    #[serde(default)]
    pub order_id: String,
    #[serde(default)]
    pub order_status: String,
}

/// A single entry of `GET /orders` and `GET /orders/{id}`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Order {
    #[serde(default)]
    pub dhan_client_id: String,
    #[serde(default)]
    pub order_id: String,
    #[serde(default)]
    pub correlation_id: Option<String>,
    #[serde(default)]
    pub order_status: String,
    #[serde(default)]
    pub transaction_type: String,
    #[serde(default)]
    pub exchange_segment: String,
    #[serde(default)]
    pub product_type: String,
    #[serde(default)]
    pub order_type: String,
    #[serde(default)]
    pub validity: String,
    #[serde(default)]
    pub trading_symbol: String,
    #[serde(default)]
    pub security_id: String,
    #[serde(default)]
    pub quantity: i64,
    #[serde(default)]
    pub disclosed_quantity: i64,
    #[serde(default)]
    pub price: f64,
    #[serde(default)]
    pub trigger_price: f64,
    #[serde(default)]
    pub after_market_order: bool,
    #[serde(default)]
    pub leg_name: Option<String>,
    #[serde(default)]
    pub create_time: String,
    #[serde(default)]
    pub update_time: String,
    #[serde(default)]
    pub exchange_time: String,
    #[serde(default)]
    pub drv_expiry_date: Option<String>,
    #[serde(default)]
    pub drv_option_type: Option<String>,
    #[serde(default)]
    pub drv_strike_price: f64,
    #[serde(default)]
    pub oms_error_code: Option<String>,
    #[serde(default)]
    pub oms_error_description: Option<String>,
    #[serde(default)]
    pub remaining_quantity: i64,
    #[serde(default)]
    pub average_traded_price: f64,
    #[serde(default)]
    pub filled_qty: i64,
}

/// A single entry of `GET /trades` and `GET /trades/{order-id}`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Trade {
    #[serde(default)]
    pub dhan_client_id: String,
    #[serde(default)]
    pub order_id: String,
    #[serde(default)]
    pub exchange_order_id: String,
    #[serde(default)]
    pub exchange_trade_id: String,
    #[serde(default)]
    pub transaction_type: String,
    #[serde(default)]
    pub exchange_segment: String,
    #[serde(default)]
    pub product_type: String,
    #[serde(default)]
    pub order_type: String,
    #[serde(default)]
    pub trading_symbol: String,
    #[serde(default)]
    pub security_id: String,
    #[serde(default)]
    pub traded_quantity: i64,
    #[serde(default)]
    pub traded_price: f64,
    #[serde(default)]
    pub create_time: String,
    #[serde(default)]
    pub update_time: String,
    #[serde(default)]
    pub exchange_time: String,
    #[serde(default)]
    pub drv_expiry_date: Option<String>,
    #[serde(default)]
    pub drv_option_type: Option<String>,
    #[serde(default)]
    pub drv_strike_price: f64,
}

// ---------------------------------------------------------------------------
// Trading: portfolio, funds
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Holding {
    #[serde(default)]
    pub exchange: String,
    #[serde(default)]
    pub trading_symbol: String,
    #[serde(default)]
    pub security_id: String,
    #[serde(default)]
    pub isin: String,
    #[serde(default)]
    pub total_qty: i64,
    #[serde(default)]
    pub dp_qty: i64,
    #[serde(default)]
    pub t1_qty: i64,
    #[serde(default)]
    pub available_qty: i64,
    #[serde(default)]
    pub collateral_qty: i64,
    #[serde(default)]
    pub avg_cost_price: f64,
    /// Live market price, when the broker includes it in the holdings payload.
    /// Optional so an API that omits it still deserializes (defaults to 0, and
    /// the Account view falls back to the shared LTP cache / avg cost).
    #[serde(default)]
    pub last_price: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Position {
    #[serde(default)]
    pub dhan_client_id: String,
    #[serde(default)]
    pub trading_symbol: String,
    #[serde(default)]
    pub security_id: String,
    #[serde(default)]
    pub position_type: String,
    #[serde(default)]
    pub exchange_segment: String,
    #[serde(default)]
    pub product_type: String,
    #[serde(default)]
    pub buy_avg: f64,
    #[serde(default)]
    pub buy_qty: i64,
    #[serde(default)]
    pub cost_price: f64,
    #[serde(default)]
    pub sell_avg: f64,
    #[serde(default)]
    pub sell_qty: i64,
    #[serde(default)]
    pub net_qty: i64,
    #[serde(default)]
    pub realized_profit: f64,
    #[serde(default)]
    pub unrealized_profit: f64,
    #[serde(default)]
    pub multiplier: i64,
    #[serde(default)]
    pub day_buy_qty: i64,
    #[serde(default)]
    pub day_sell_qty: i64,
    #[serde(default)]
    pub drv_expiry_date: Option<String>,
    #[serde(default)]
    pub drv_option_type: Option<String>,
    #[serde(default)]
    pub drv_strike_price: f64,
    #[serde(default)]
    pub cross_currency: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ConvertPositionRequest {
    pub dhan_client_id: String,
    pub from_product_type: ProductType,
    pub exchange_segment: ExchangeSegment,
    pub position_type: PositionType,
    pub security_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trading_symbol: Option<String>,
    pub convert_qty: i64,
    pub to_product_type: ProductType,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct FundLimit {
    #[serde(default)]
    pub dhan_client_id: String,
    /// Dhan spells this field "availabelBalance" in the API contract.
    #[serde(default, alias = "availableBalance")]
    pub availabel_balance: f64,
    #[serde(default)]
    pub sod_limit: f64,
    #[serde(default)]
    pub collateral_amount: f64,
    #[serde(default)]
    pub receiveable_amount: f64,
    #[serde(default)]
    pub utilized_amount: f64,
    #[serde(default)]
    pub blocked_payout_amount: f64,
    #[serde(default)]
    pub withdrawable_balance: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MarginRequest {
    pub dhan_client_id: String,
    pub exchange_segment: ExchangeSegment,
    pub transaction_type: TransactionType,
    pub quantity: i64,
    pub product_type: ProductType,
    pub security_id: String,
    pub price: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trigger_price: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MarginResponse {
    #[serde(default)]
    pub total_margin: f64,
    #[serde(default)]
    pub span_margin: f64,
    #[serde(default)]
    pub exposure_margin: f64,
    #[serde(default)]
    pub available_balance: f64,
    #[serde(default)]
    pub variable_margin: f64,
    #[serde(default)]
    pub insufficient_balance: f64,
    #[serde(default)]
    pub brokerage: f64,
    #[serde(default)]
    pub leverage: String,
}

// ---------------------------------------------------------------------------
// Profile
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    #[serde(default)]
    pub dhan_client_id: String,
    #[serde(default)]
    pub token_validity: String,
    #[serde(default)]
    pub active_segment: String,
    #[serde(default)]
    pub ddpi: String,
    #[serde(default)]
    pub mtf: String,
    #[serde(default)]
    pub data_plan: String,
    #[serde(default)]
    pub data_validity: String,
}

// ---------------------------------------------------------------------------
// Data: historical candles
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CandleData {
    #[serde(default)]
    pub open: Vec<f64>,
    #[serde(default)]
    pub high: Vec<f64>,
    #[serde(default)]
    pub low: Vec<f64>,
    #[serde(default)]
    pub close: Vec<f64>,
    #[serde(default)]
    pub volume: Vec<f64>,
    #[serde(default)]
    pub timestamp: Vec<f64>,
    #[serde(default)]
    pub open_interest: Vec<f64>,
}

/// One OHLCV row, ready to feed the chart engine.
#[derive(Debug, Clone, Copy, Default)]
pub struct CandleRow {
    pub time: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
    pub open_interest: f64,
}

impl CandleData {
    /// Zip the parallel arrays into rows, dropping any index Dhan omitted.
    pub fn rows(&self) -> Vec<CandleRow> {
        let n = self
            .timestamp
            .len()
            .min(self.open.len())
            .min(self.high.len())
            .min(self.low.len())
            .min(self.close.len());
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            out.push(CandleRow {
                time: self.timestamp[i] as i64,
                open: self.open[i],
                high: self.high[i],
                low: self.low[i],
                close: self.close[i],
                volume: self.volume.get(i).copied().unwrap_or(0.0),
                open_interest: self.open_interest.get(i).copied().unwrap_or(0.0),
            });
        }
        out
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoricalRequest {
    pub security_id: String,
    pub exchange_segment: ExchangeSegment,
    pub instrument: Instrument,
    /// Dhan requires this field on `/charts/historical` even for cash instruments
    /// and indices (0 = no expiry); omitting it fails with DH-905.
    #[serde(default)]
    pub expiry_code: i32,
    #[serde(default)]
    pub oi: bool,
    pub from_date: String,
    pub to_date: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntradayRequest {
    pub security_id: String,
    pub exchange_segment: ExchangeSegment,
    pub instrument: Instrument,
    /// Minutes per candle: one of 1, 5, 15, 25, 60.
    pub interval: String,
    #[serde(default)]
    pub oi: bool,
    pub from_date: String,
    pub to_date: String,
}

// ---------------------------------------------------------------------------
// Data: option chain
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Greeks {
    #[serde(default)]
    pub delta: f64,
    #[serde(default)]
    pub theta: f64,
    #[serde(default)]
    pub gamma: f64,
    #[serde(default)]
    pub vega: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OptionLeg {
    #[serde(default)]
    pub average_price: f64,
    #[serde(default)]
    pub greeks: Option<Greeks>,
    #[serde(default)]
    pub implied_volatility: f64,
    #[serde(default)]
    pub last_price: f64,
    #[serde(default)]
    pub oi: f64,
    #[serde(default)]
    pub previous_close_price: f64,
    #[serde(default)]
    pub previous_oi: f64,
    #[serde(default)]
    pub previous_volume: f64,
    #[serde(default)]
    pub security_id: i64,
    #[serde(default)]
    pub top_ask_price: f64,
    #[serde(default)]
    pub top_ask_quantity: i64,
    #[serde(default)]
    pub top_bid_price: f64,
    #[serde(default)]
    pub top_bid_quantity: i64,
    #[serde(default)]
    pub volume: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StrikeLegs {
    #[serde(default)]
    pub ce: Option<OptionLeg>,
    #[serde(default)]
    pub pe: Option<OptionLeg>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OptionChainData {
    #[serde(default)]
    pub last_price: f64,
    /// Map of strike price string ("25650.000000") to its CE/PE legs.
    #[serde(default)]
    pub oc: std::collections::BTreeMap<String, StrikeLegs>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OptionChainResponse {
    #[serde(default)]
    pub data: OptionChainData,
    #[serde(default)]
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OptionChainRequest {
    pub underlying_scrip: i64,
    pub underlying_seg: ExchangeSegment,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expiry: Option<String>,
}

// ---------------------------------------------------------------------------
// Data: market quote
// ---------------------------------------------------------------------------

/// Request body for `/marketfeed/{ltp,ohlc,quote}`: a map of exchange segment
/// to a list of security ids.
pub type SegmentInstruments = std::collections::BTreeMap<String, Vec<i64>>;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Ohlc {
    #[serde(default)]
    pub open: f64,
    #[serde(default)]
    pub close: f64,
    #[serde(default)]
    pub high: f64,
    #[serde(default)]
    pub low: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DepthLevel {
    #[serde(default)]
    pub quantity: i64,
    #[serde(default)]
    pub orders: i64,
    #[serde(default)]
    pub price: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Depth {
    #[serde(default)]
    pub buy: Vec<DepthLevel>,
    #[serde(default)]
    pub sell: Vec<DepthLevel>,
}

/// A single instrument inside a `/marketfeed/quote` response. Only the fields we
/// consume are typed; everything else Dhan sends is ignored.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub struct QuoteEntry {
    #[serde(default)]
    pub last_price: f64,
    #[serde(default)]
    pub last_quantity: i64,
    #[serde(default)]
    pub last_trade_time: String,
    #[serde(default)]
    pub average_price: f64,
    #[serde(default)]
    pub volume: f64,
    #[serde(default)]
    pub net_change: f64,
    /// Previous session close. Dhan's `/marketfeed/quote` exposes it for many
    /// instruments; when absent (0) the caller derives it from `net_change`.
    #[serde(default)]
    pub previous_close_price: f64,
    #[serde(default)]
    pub buy_quantity: i64,
    #[serde(default)]
    pub sell_quantity: i64,
    #[serde(default)]
    pub lower_circuit_limit: f64,
    #[serde(default)]
    pub upper_circuit_limit: f64,
    #[serde(default)]
    pub oi: f64,
    #[serde(default)]
    pub oi_day_high: f64,
    #[serde(default)]
    pub oi_day_low: f64,
    #[serde(default)]
    pub ohlc: Option<Ohlc>,
    #[serde(default)]
    pub depth: Option<Depth>,
}

/// A `/marketfeed` response: `data.{segment}.{securityId}` plus a status.
pub type MarketFeedResponse =
    std::collections::BTreeMap<String, std::collections::BTreeMap<String, QuoteEntry>>;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Envelope<T> {
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub data: Option<T>,
    #[serde(default)]
    pub auth_error: Option<String>,
}
