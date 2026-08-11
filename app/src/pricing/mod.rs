use onboarding::CreditPackOption;
use warpui::{Entity, ModelContext, SingletonEntity};

#[derive(Debug, Clone)]
pub struct AddonCreditsOption {
    pub credits: i32,
    pub price_usd_cents: i32,
}

impl AddonCreditsOption {
    pub fn rate(&self) -> f32 {
        self.price_usd_cents as f32 / self.credits as f32
    }

    /// Returns the purchase price in cents after applying a plan surcharge
    /// expressed in basis points (1000 bps = +10%). `price_usd_cents` always
    /// carries the list price; plans whose `PurchaseAddOnCreditsPolicy` has a
    /// non-zero `price_premium_bps` pay a premium on top of it. The surcharge
    /// is rounded up to the next cent using the same integer math as the
    /// server so displayed prices always match what is charged.
    pub fn price_usd_cents_with_premium(&self, premium_bps: i32) -> i32 {
        if premium_bps <= 0 {
            return self.price_usd_cents;
        }
        let price = self.price_usd_cents as i64;
        let surcharge = (price * premium_bps as i64 + 9_999) / 10_000;
        (price + surcharge) as i32
    }
}

#[derive(Debug, Clone)]
pub struct OveragesPricing {
    pub price_per_request_usd_cents: i32,
}

#[derive(Debug, Clone)]
pub struct PricingInfo {
    pub plans: Vec<PlanPricing>,
    pub overages: OveragesPricing,
    pub addon_credits_options: Vec<AddonCreditsOption>,
    pub promotion_message: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PlanPricing {
    pub plan: StripeSubscriptionPlan,
    pub monthly_plan_price_per_month_usd_cents: i32,
    pub yearly_plan_price_per_month_usd_cents: i32,
    pub request_limit: Option<i32>,
    pub max_team_size: Option<i32>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StripeSubscriptionPlan {
    Business,
    Lightspeed,
    Pro,
    Team,
    Turbo,
    Build,
    BuildBusiness,
    BuildMax,
    Other(String),
}

/// Converts the server's add-on credit packs into the display options shown on
/// the onboarding offer slide.
///
/// `premium_bps` is the viewer's `PurchaseAddOnCreditsPolicy` surcharge (see
/// [`crate::workspaces::workspace::PurchaseAddOnCreditsPolicy::effective_premium_bps`]),
/// applied with the same integer math the server charges with, so the price we
/// show is the price billed. Savings are computed against the smallest pack's
/// per-credit list rate — the premium scales every pack equally, so it doesn't
/// change the relative volume discount.
pub fn onboarding_credit_pack_options(
    options: &[AddonCreditsOption],
    premium_bps: i32,
) -> Vec<CreditPackOption> {
    let base_rate = options.first().map_or(0., |option| option.rate());
    options
        .iter()
        .map(|option| {
            let savings_percent = if base_rate > 0. {
                (((base_rate - option.rate()) / base_rate) * 100.)
                    .round()
                    .max(0.) as u32
            } else {
                0
            };
            CreditPackOption {
                credits: option.credits,
                price_usd_cents: option.price_usd_cents_with_premium(premium_bps),
                savings_percent,
            }
        })
        .collect()
}

/// 服务端价格信息的全局模型。
///
/// Zap 中它是本地 no-op stub:OSS channel 没有云端服务推送价格数据,
/// 所以进程生命周期内 `pricing_info` 通常保持 `None`,所有 getter 都返回 `None`。
/// 模型暂时保留给少量请求用量和计费兼容调用点,后续云端清理完成后可整段删除。
#[derive(Debug)]
pub struct PricingInfoModel {
    pricing_info: Option<PricingInfo>,
}

impl PricingInfoModel {
    pub fn new() -> Self {
        Self { pricing_info: None }
    }

    /// Updates the model with the latest pricing information from the server.
    pub fn update_pricing_info(&mut self, pricing_info: PricingInfo, ctx: &mut ModelContext<Self>) {
        self.pricing_info = Some(pricing_info);
        ctx.emit(PricingInfoModelEvent::PricingInfoUpdated);
    }

    /// Returns the pricing for a specific plan.
    pub fn plan_pricing(&self, plan: &StripeSubscriptionPlan) -> Option<&PlanPricing> {
        self.pricing_info
            .as_ref()?
            .plans
            .iter()
            .find(|p| &p.plan == plan)
    }

    pub fn addon_credits_options(&self) -> Option<&[AddonCreditsOption]> {
        self.pricing_info
            .as_ref()
            .map(|info| info.addon_credits_options.as_slice())
    }

    pub fn promotion_message(&self) -> Option<&str> {
        self.pricing_info.as_ref()?.promotion_message.as_deref()
    }
}

impl Default for PricingInfoModel {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub enum PricingInfoModelEvent {
    PricingInfoUpdated,
}

impl Entity for PricingInfoModel {
    type Event = PricingInfoModelEvent;
}

impl SingletonEntity for PricingInfoModel {}

#[cfg(test)]
#[path = "pricing_tests.rs"]
mod tests;
