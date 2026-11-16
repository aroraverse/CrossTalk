"""A tiny checkout service used for the Crosstalk Claude <-> Codex demo.

Standard library only. Money is computed with Decimal internally (inputs may
still be float/int; they are converted via str()), and order_total returns a
float rounded half-up to 2 places, so the public API is unchanged.
"""

from dataclasses import dataclass
from datetime import date, datetime
from decimal import ROUND_DOWN, ROUND_HALF_UP, Decimal
from typing import Optional


TAX_RATE = Decimal("0.18")  # 18% GST
_CENT = Decimal("0.01")


class ExpiredCouponError(ValueError):
    """Raised when a coupon is used after its expiry date."""


@dataclass
class Item:
    name: str
    unit_price: float  # rupees
    qty: int


@dataclass
class Coupon:
    code: str
    percent_off: float  # e.g. 20 means 20%
    expires: date
    max_discount: Optional[float] = None  # rupees; None = no cap


def _dec(value, label: str) -> Decimal:
    try:
        d = Decimal(str(value))
    except Exception:
        raise ValueError(f"{label} is not a number: {value!r}")
    if not d.is_finite():
        raise ValueError(f"{label} must be finite: {value!r}")
    return d


def _money(d: Decimal) -> Decimal:
    return d.quantize(_CENT, rounding=ROUND_HALF_UP)


def _subtotal(items: list[Item]) -> Decimal:
    total = Decimal(0)
    for item in items:
        price = _dec(item.unit_price, "unit_price")
        if price < 0:
            raise ValueError("unit_price must be >= 0")
        if not isinstance(item.qty, int) or isinstance(item.qty, bool) or item.qty < 0:
            raise ValueError("qty must be an integer >= 0")
        total += price * item.qty
    return total


def subtotal(items: list[Item]) -> float:
    """Sum of unit price x quantity."""
    return float(_subtotal(items))


def _apply_coupon(amount: Decimal, coupon: Optional[Coupon],
                  today: date) -> Decimal:
    if amount < 0:
        raise ValueError("amount must be >= 0")
    if coupon is None:
        return amount
    if (not isinstance(today, date) or isinstance(today, datetime)
            or not isinstance(coupon.expires, date)
            or isinstance(coupon.expires, datetime)):
        raise ValueError("expires and today must be dates")
    # Precedence: expiry is checked before field validation.
    if coupon.expires < today:
        raise ExpiredCouponError(
            f"coupon {coupon.code} expired on {coupon.expires.isoformat()}")
    percent = _dec(coupon.percent_off, "percent_off")
    if not Decimal(0) <= percent <= Decimal(100):
        raise ValueError("percent_off must be between 0 and 100")
    discount = amount * percent / 100
    if coupon.max_discount is not None:
        cap = _dec(coupon.max_discount, "max_discount")
        if cap < 0:
            raise ValueError("max_discount must be >= 0")
        # Cap is rounded DOWN to whole paise so rounding can never exceed it.
        discount = min(_money(discount), cap.quantize(_CENT, rounding=ROUND_DOWN))
    discount = min(_money(discount), amount)
    return amount - discount


def apply_coupon(amount: float, coupon: Optional[Coupon],
                 today: Optional[date] = None) -> float:
    """Apply a percentage coupon (with optional cap) to an amount."""
    result = _apply_coupon(_dec(amount, "amount"), coupon,
                           today or date.today())
    return float(result)


def order_total(items: list[Item], coupon: Optional[Coupon] = None,
                today: Optional[date] = None) -> float:
    """Final amount: subtotal, minus capped coupon (pre-tax), plus tax."""
    today = today or date.today()
    discounted = _apply_coupon(_subtotal(items), coupon, today)
    return float(_money(discounted * (1 + TAX_RATE)))
