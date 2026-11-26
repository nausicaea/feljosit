# User guide

## The method

Feljosit uses the envelope method. You give each franc a job:

1. Income goes to "Ready to Assign".
2. You assign this money to categories (for example "Rent", "Groceries").
3. When you spend money, the "Available" amount of the category decreases.
4. At the end of the month, a positive "Available" amount moves to the next month.

Your goal: "Ready to Assign" is 0.00, and no category has a negative "Available" amount.

## Procedure: add your accounts

1. Click "Accounts".
2. Type a name, for example "UBS private account".
3. Select the type.
4. Type the IBAN. The import uses the IBAN to find the account.
5. If you do not import a statement, type the starting balance and the date.
6. Click "Add the account".

The types private account, savings account, credit card and cash are budget
accounts. The types pillar 3a, investment and loan are tracking accounts.
A tracking account shows only the balance. It is not part of the budget.

An account in a currency other than CHF must be a tracking account.

You do not have to add an account before you import a statement. If no account
has the IBAN of the statement, the import adds the account. See
"Procedure: import a statement".

## Procedure: edit an account

1. Click "Accounts".
2. Click "Edit" in the row of the account.
3. Change the name, the type, or "Budget or tracking".
4. Click "Save".

You cannot change the IBAN or the currency.

CAUTION: If you change a budget account to a tracking account, its transactions
lose their categories. If you change a tracking account to a budget account, its
starting balance goes to "Ready to Assign". You must categorise its other
transactions.

When you change "Budget or tracking", the application finds the transfers
between this account and your other accounts again. A movement between two budget
accounts, or between two tracking accounts, is a transfer. A movement between a
budget account and a tracking account needs a category.

## Procedure: get a camt.053 statement from your bank

1. Log in to your e-banking.
2. Find the account statements or the document export.
3. Select the format "camt.053" (sometimes "ISO 20022" or "XML").
4. Download the file. A ZIP file is also correct.

The name of the function is different for each bank.
Ask your bank if you cannot find it.

## Procedure: import a statement

1. Click "Import".
2. Select the file.
3. Keep "Automatic" for the account, or select the account. Keep "Automatic" if
   the file has statements for more than one account.
4. Click "Import".
5. Read the result.

The import does these steps:

- It finds the account from the IBAN of each statement. If no account has the
  IBAN, it adds a private account. The name comes from the statement, for example
  "Account CH89 0023 5000 0123 4567 8". An account in CHF is a budget account.
  An account in a different currency is a tracking account. Change the name and
  the type after the import (see "Procedure: edit an account").
- It ignores entries that are not booked.
- It ignores transactions that you imported before.
- It uses the opening balance of the first statement as the starting balance.
- It gives categories to known transactions.
- It finds transfers between your budget accounts. If you add an account later,
  the application finds the transfers to this account in the transactions that
  you imported before.
- It finds transfers that have no IBAN, for example a payment to your credit card.
  Two transactions are a transfer if they are on two different accounts, have
  the opposite amount, and are booked at most 4 days apart. Both must be
  uncategorised, and the pair must be clear: if there are two possible partners,
  the application does not decide. The result shows the number of these
  transfers.
  "Apply the rules to uncategorised transactions" on the categories page also
  finds these transfers in the transactions that you imported before.
- It compares the closing balance of the statement with the account balance.

CAUTION: If the closing balance does not agree, transactions are possibly missing.
Import the older statements too.

CAUTION: Two unrelated transactions with the same amount can become a wrong
transfer. If a transfer is wrong, select the correct category for both
transactions of the pair.

## Procedure: categorise transactions

1. Click "Transactions".
2. Select "Only uncategorised".
3. For each transaction, select a category. The change is saved at once.

For a transfer between your accounts, select "Transfer". A transfer from a budget
account to a tracking account (for example to pillar 3a) needs a category,
because the money leaves the budget.

## Procedure: move a category into a different group

1. Click "Categories".
2. Put the mouse pointer on ⠿ at the left of the category.
3. Hold the mouse button and drag the category to the new position. The new
   position can be in a different group or in the same group.
4. Release the mouse button.

The application saves the change immediately. The budget page shows the
categories in the same groups and in the same order.

Alternative: select the new group in the list of the category. The category moves
to the end of the group. Use this method on a phone or a tablet, or with the
keyboard.

The transactions, the assigned amounts and the rules of the category do not change.

## Procedure: make a rule

1. Click "Categories".
2. Go to "Categorisation rules".
3. Select the field: counterparty, counterparty IBAN or description.
4. Select "contains" or "equals".
5. Type the text, for example "Migros".
6. Select the category.
7. Click "Add the rule".
8. To apply the rule to the existing transactions, click "Apply the rules to uncategorised transactions".

The rule with the lowest priority number applies first. If no rule applies,
the application uses the category of the last transaction with the same counterparty.

## Procedure: assign money

1. Click "Budget".
2. Select the month with the arrows.
3. In the "Assigned" column, type an amount. You can type `1'200` or `1200.50`.
4. Press Enter.

If "Ready to Assign" is negative, you assigned more money than you have.
Decrease the assigned amounts.

If a category has a negative "Available" amount, you overspent. Move money from
a different category, or the next month has less money to assign.

## Reports

Click "Reports". Select the period:

- Click a preset period (3, 6, 12 or 24 months). A preset period includes the current month.
- Or, click "All". The period starts in the month of your first transaction and includes the current month. If your transactions are more than 20 years old, the page shows only the last 20 years.
- Or, type the dates in "From" and "To" and click "Show". The two dates are included. The maximum period is 20 years.

The page shows:

- Income, expenses, net amount and savings rate.
- Income and expenses for each month.
- Spending by category.
- Net worth (all accounts, one chart for each currency).

To compare the categories over time, click "Spending by category" below the
title. To compare the counterparties, click "Spending by counterparty". See
"Procedure: find expenses that you can reduce" and "Procedure: compare
counterparties". All report pages use the same period.

The reports use only budget accounts, except the net worth. Transfers are not included.

The charts show one value for each month. If the period starts or stops in a month, that month includes only the transactions in the period. The net worth of the last month is the net worth on the "To" date.

You can save or share the address of a custom period, for example `/reports?from=2026-01-01&to=2026-06-30`.

## Procedure: find expenses that you can reduce

1. Click "Reports".
2. Click "Spending by category".
3. Select a period of 6 months or more. Then the page shows the change.
4. Click "Largest total" or "Largest increase" to sort the categories.
5. Click a category to see the details.

The table shows one row for each expense category with spending in the period:

- "Total", "Share" and "Per month": the spending of the category in the period.
- "Last 3 months": the average spending per month in the last 3 months.
- "Change": the average of the last 3 months against the average of the months
  before. An increase of 10 % or more is red. A decrease of 10 % or more is green.
- One column for each month. For a period of more than 12 months, one column for
  each quarter. For a period of more than 36 months, one column for each year.
  A quarter or year column shows the average spending per month. Thus a quarter
  that is only partly in the period is comparable with the others.
- A colour shows that the spending is more than the average per month of the
  category: light for 10 % more, dark for 100 % or more.

The details of a category show the spending per month and the counterparties with
the most spending. Use them to find where the money goes, for example the shop or
the restaurant.

The report uses the same transactions as "Spending by category" on the overview:
expense categories and uncategorised outflows of budget accounts, without
transfers. A refund decreases the spending of its category.

TIP: Categorise all transactions before you use the report. Uncategorised
outflows are in one row, "Uncategorised".

## Procedure: compare counterparties

1. Click "Reports".
2. Click "Spending by counterparty".
3. Select the period.
4. Click "Largest total" or "Largest increase" to sort the counterparties.
5. Click a counterparty to see the details.

The table is the same as in "Spending by category", but it has one row for each
counterparty: for example a shop, a restaurant or a landlord. It shows the 30
counterparties with the most spending. The other counterparties are in one row,
"Other counterparties". Thus the total is the same as in "Spending by category".

The small text below the name shows the category with the most spending for the
counterparty. The details of a counterparty show its spending per month and its
spending in each category.

To go from a category to a counterparty, click the counterparty in the details
of the category.

NOTE: The report uses the counterparty name of the transaction. If your bank
writes the name of the same shop in different ways (for example with the
location), the shop has more than one row.

## Procedure: change your password

1. Click your username.
2. Type the current password.
3. Type the new password two times.
4. Click "Change the password".

All your other sessions close.

## Procedure: reset a forgotten password

1. Open a terminal on the computer with the database.
2. Type `uv run feljosit set-password USERNAME`.
3. Type the new password two times.

This command also unlocks a locked account.
