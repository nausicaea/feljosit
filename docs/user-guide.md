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
3. Keep "Automatic" for the account, or select the account.
4. Click "Import".
5. Read the result.

The import does these steps:

- It ignores entries that are not booked.
- It ignores transactions that you imported before.
- It uses the opening balance of the first statement as the starting balance.
- It gives categories to known transactions.
- It finds transfers between your budget accounts.
- It compares the closing balance of the statement with the account balance.

CAUTION: If the closing balance does not agree, transactions are possibly missing.
Import the older statements too.

## Procedure: categorise transactions

1. Click "Transactions".
2. Select "Only uncategorised".
3. For each transaction, select a category. The change is saved at once.

For a transfer between your accounts, select "Transfer". A transfer from a budget
account to a tracking account (for example to pillar 3a) needs a category,
because the money leaves the budget.

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

Click "Reports". Select the period. The page shows:

- Income, expenses, net amount and savings rate.
- Income and expenses for each month.
- Spending by category.
- Net worth (all accounts, one chart for each currency).

The reports use only budget accounts, except the net worth. Transfers are not included.

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
