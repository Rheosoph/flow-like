Create the two roles and prove their boundaries from the second account.

1. In the practice app's Team area, create an Operator role using the levels from the previous lesson. Add only other read permissions that the intended interface actually needs and record each addition.
2. Create the custom table-reader role with `ReadDatabase`. Do not add `ReadFiles` to solve a navigation problem without checking the wider access it grants.
3. Assign one test role at a time to the second account. Remove other roles that could mask a missing permission.
4. Sign in as that account and perform each applicable operation in the access matrix.
5. Record the result and one observation, such as a denied request or an unavailable editing control. A hidden button alone does not prove a server operation is denied; attempt the operation through the supported interface where possible.

Use an invite link only for this test group. Its default role determines the access every redeemer receives. Start with the narrowest useful role, then grant extra access deliberately. Remove the test link when the exercise ends.

Completion: the three operator read/run operations succeed and both edit operations fail. The reader can inspect the table and cannot read the app file. Investigate unexpected access before continuing, including inherited roles and elevation.
