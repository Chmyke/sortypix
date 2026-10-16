# Security

Report a vulnerability privately through GitHub: open the **Security** tab of this repository and
choose **Report a vulnerability**. Please do not open a public issue for a security problem.

Only the latest released version is supported.

For context when assessing a report: Sortypix never uses the network. It reads one folder of
photos and only moves files into its `keep/` and `rejected/` subfolders; it never deletes or
overwrites a photo. Its own settings and preview cache live under `%LOCALAPPDATA%`.
