# Third-Party Notices — ODBC drivers in the server executor images

The cloud executor images install ODBC drivers from distribution and vendor package
repositories so the database nodes can connect by driver name (`Driver={…}` in a
connection string). Flow-Like does not link them; unixODBC (see `unixodbc.md`) loads them
at run time. Each driver keeps its own license; the license files ship inside the images
at the paths below.

| Driver (name in odbcinst.ini) | Package | Images | License | License file in the image |
|---|---|---|---|---|
| ODBC Driver 18 for SQL Server | `msodbcsql18` (Microsoft, packages.microsoft.com) | all executor images | Microsoft Software License Terms, https://aka.ms/odbc18eula (installed with `ACCEPT_EULA=Y`) | `/usr/share/doc/msodbcsql18/LICENSE.txt` |
| PostgreSQL | `postgresql-odbc` (Amazon Linux 2023), `odbc-postgresql` (Debian 12) — psqlODBC | all executor images | LGPL-2.0-or-later | Debian: `/usr/share/doc/odbc-postgresql/copyright`; Amazon Linux: the package ships no license file (RPM license field `LGPL-2.0-or-later`), see https://odbc.postgresql.org/license.html |
| MariaDB | `odbc-mariadb` (Debian 12) — MariaDB Connector/ODBC | Azure, GCP and Kubernetes images | LGPL-2.1-or-later | `/usr/share/doc/odbc-mariadb/copyright` |

Sources:
- psqlODBC: https://odbc.postgresql.org/
- MariaDB Connector/ODBC: https://github.com/mariadb-corporation/mariadb-connector-odbc
- Microsoft ODBC Driver 18 for SQL Server: https://learn.microsoft.com/sql/connect/odbc/download-odbc-driver-for-sql-server
