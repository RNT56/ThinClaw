# GitHub Tool

> Integration for managing repositories, issues, pull requests, and CI/CD workflows.

## Authentication

The GitHub tool uses a **Personal Access Token (PAT)** — not OAuth. This is the
correct approach when the agent has its own GitHub account.

### Setup Steps

1. **Create a GitHub account for your agent** (or use an existing one)
   - Go to [github.com/signup](https://github.com/signup)
   - Use a descriptive username (e.g. `thinclaw-agent`, `myorg-bot`)

2. **Generate a Personal Access Token**
   - Log in as the agent account
   - Go to **Settings → Developer Settings → Personal access tokens → Fine-grained tokens**
   - Or for classic tokens: [github.com/settings/tokens](https://github.com/settings/tokens)
   - Click **"Generate new token"**

3. **Required permissions (scopes)**

   | Scope | Why |
   |-------|-----|
   | `repo` | Read/write access to repositories (issues, PRs, code) |
   | `workflow` | Trigger and manage GitHub Actions |
   | `read:org` | List org repos and members |

   For fine-grained tokens, enable:
   - Repository access: All or specific repos
   - Repository permissions: Issues (R/W), Pull requests (R/W), Contents (Read), Actions (R/W)

4. **Store the token**
   Save the PAT using your current ThinClaw secret-entry flow for `github_token` or via env-based secret management if your deployment relies on environment variables.

5. **Verify**
   ```
   You: Use the GitHub tool to get info about octocat/hello-world
   ```

### Headless / Remote Setup

If running on a headless server, PAT setup is still manual-token based. Use the same `github_token` secret flow you use for the rest of your deployment.

## Available Actions (12)

| Action | Description |
|--------|-------------|
| `get_repo` | Get repository info (owner, description, stars, language) |
| `list_repos` | List repositories for a user |
| `list_issues` | List issues with state filter and pagination |
| `create_issue` | Create a new issue with title, body, and labels |
| `get_issue` | Get a single issue by number |
| `list_pull_requests` | List PRs with state filter and pagination |
| `get_pull_request` | Get a single PR by number |
| `get_pull_request_files` | List changed files in a PR |
| `create_pr_review` | Submit a review (APPROVE, REQUEST_CHANGES, COMMENT) |
| `get_file_content` | Read file content from a repo (with optional branch/ref) |
| `trigger_workflow` | Trigger a GitHub Actions workflow dispatch |
| `get_workflow_runs` | List recent workflow runs |

## Rate Limits

- 60 requests/minute, 3600 requests/hour (enforced by capabilities)
- GitHub API also enforces 5000 requests/hour per authenticated user

## Security

- The PAT is stored encrypted in ThinClaw's secret store (AES-256-GCM)
- The WASM tool never sees the token — it's injected as a Bearer header by the host
- All API responses are scanned for leaked secrets before returning to the tool
