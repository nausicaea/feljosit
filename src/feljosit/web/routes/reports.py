"""Reports: visualisations and statistics."""

from __future__ import annotations

from datetime import date
from typing import Annotated

from fastapi import APIRouter, Query, Request
from fastapi.responses import HTMLResponse, Response
from pydantic import BeforeValidator

from feljosit.money import format_minor
from feljosit.services import reports
from feljosit.services.budget import add_months, month_start
from feljosit.web import charts
from feljosit.web.deps import AppSettings, CurrentAuth
from feljosit.web.rendering import month_short, render

router = APIRouter()

PERIODS = {"3m": 3, "6m": 6, "12m": 12, "24m": 24}
ALL_PERIOD = "all"
"""The period from the month of the first transaction to the current month."""
PERIOD_LABELS = {"3m": "3 months", "6m": "6 months", "12m": "12 months", "24m": "24 months"}
PERIOD_LABELS[ALL_PERIOD] = "All"
DEFAULT_PERIOD = "12m"
MAX_MONTHS = 240
"""The maximum length of a custom period (20 years). This limits the size of the charts."""
MIN_DATE = date(1900, 1, 1)
MAX_DATE = date(9998, 12, 31)
"""Later dates are not possible: the month calculations must stay below year 10000."""


def _empty_as_none(value: object) -> object:
    """Change an empty query value to ``None``.

    The date form sends ``from=`` when the user does not type a date.
    """
    return None if value == "" else value


def _last_day_of_month(month: date) -> date:
    return date.fromordinal(add_months(month, 1).toordinal() - 1)


def _preset_range(period: str, today: date) -> tuple[date, date]:
    """The range of a preset period: the last full months, including the current month."""
    months = PERIODS.get(period, PERIODS[DEFAULT_PERIOD])
    end_month = month_start(today)
    return add_months(end_month, -(months - 1)), _last_day_of_month(end_month)


def all_range(data_range: tuple[date, date] | None, today: date) -> tuple[date, date, bool]:
    """The range of the "All" period and ``True`` if the range was made shorter.

    The range starts in the month of the first transaction. It stops at the end of the
    current month, or at the end of the month of the last transaction if that is later.
    If the range is longer than ``MAX_MONTHS``, only the last ``MAX_MONTHS`` months are
    used. If there are no transactions, the range is the current month.
    """
    end_month = month_start(today)
    if data_range is None:
        return end_month, _last_day_of_month(end_month), False
    first, last = data_range
    end_month = max(end_month, month_start(min(last, MAX_DATE)))
    start = max(month_start(first), month_start(MIN_DATE))
    earliest = add_months(end_month, -(MAX_MONTHS - 1))
    shortened = start < earliest
    return max(start, earliest), _last_day_of_month(end_month), shortened


def custom_range(start: date | None, end: date | None, today: date) -> tuple[date, date]:
    """The range of a custom period. Raise ``ValueError`` if the range is not valid.

    If there is no end date, the end is today. If there is no start date, the start
    is the first day of the month 11 months before the end month.
    """
    if end is None:
        end = today
    if not MIN_DATE <= end <= MAX_DATE or not (start is None or MIN_DATE <= start <= MAX_DATE):
        raise ValueError(f"Type dates from {MIN_DATE:%d.%m.%Y} to {MAX_DATE:%d.%m.%Y}.")
    if start is None:
        start = max(add_months(month_start(end), -11), MIN_DATE)
    if start > end:
        raise ValueError("The start date must be on or before the end date.")
    months = (end.year - start.year) * 12 + end.month - start.month + 1
    if months > MAX_MONTHS:
        raise ValueError(f"The period must be a maximum of {MAX_MONTHS // 12} years.")
    return start, end


OptionalDate = Annotated[date | None, BeforeValidator(_empty_as_none)]


def _month_labels(months: list[date]) -> tuple[int, int]:
    """The label step and offset for a chart with one value for each month.

    The labels are on calendar boundaries, for example January, April, July and
    October for a step of 3 months.
    """
    step = charts.month_label_step(len(months))
    if not months:
        return step, 0
    return step, months[0].year * 12 + months[0].month - 1


def _axis_formatter(currency: str) -> charts.Formatter:
    """Format axis labels in whole currency units (for example whole francs)."""

    def fmt(value: float) -> str:
        return format_minor(int(value), currency).rsplit(".", 1)[0]

    return fmt


@router.get("/reports", response_class=HTMLResponse)
def reports_page(
    request: Request,
    auth: CurrentAuth,
    settings: AppSettings,
    period: Annotated[str, Query(max_length=3)] = DEFAULT_PERIOD,
    date_from: Annotated[OptionalDate, Query(alias="from")] = None,
    date_to: Annotated[OptionalDate, Query(alias="to")] = None,
) -> Response:
    today = date.today()
    error: str | None = None
    custom = date_from is not None or date_to is not None
    shortened = False
    if period not in PERIOD_LABELS:
        period = DEFAULT_PERIOD
    if custom:
        try:
            start, end = custom_range(date_from, date_to, today)
        except ValueError as exc:
            error = str(exc)
            custom = False
    if not custom and period == ALL_PERIOD:
        data_range = reports.transaction_date_range(auth.db, auth.user.id)
        start, end, shortened = all_range(data_range, today)
    elif not custom:
        start, end = _preset_range(period, today)
    currency = settings.budget_currency

    fmt_axis = _axis_formatter(currency)
    flows = reports.monthly_flows(auth.db, auth.user.id, start, end)
    labels = [month_short(f.month) for f in flows]
    label_step, label_offset = _month_labels([f.month for f in flows])
    flow_chart = charts.grouped_bars(
        labels,
        [
            ("bar-income", [f.income for f in flows]),
            ("bar-expense", [-f.expenses for f in flows]),
        ],
        fmt_axis,
        label_step,
        label_offset,
    )
    spending = reports.spending_by_category(auth.db, auth.user.id, start, end)
    spending_rows = charts.horizontal_bars(
        [(s.name, s.total) for s in spending[:15]],
        lambda v: format_minor(int(v), currency),
    )
    worth = reports.net_worth(auth.db, auth.user.id, start, end)
    worth_charts = {
        cur: charts.line(
            [month_short(m) for m, _ in series],
            [v for _, v in series],
            _axis_formatter(cur),
            label_step,
            label_offset,
        )
        for cur, series in worth.items()
    }
    return render(
        request,
        "reports.html",
        auth,
        period=None if custom else period,
        periods=PERIOD_LABELS,
        shortened=shortened,
        max_years=MAX_MONTHS // 12,
        custom=custom,
        error=error,
        form_from=date_from.isoformat() if date_from else start.isoformat(),
        form_to=date_to.isoformat() if date_to else end.isoformat(),
        start=start,
        end=end,
        flows=flows,
        flow_chart=flow_chart,
        spending_rows=spending_rows,
        spending_total=sum(s.total for s in spending),
        summary=reports.summary(flows),
        worth_charts=worth_charts,
        status_code=400 if error else 200,
    )
