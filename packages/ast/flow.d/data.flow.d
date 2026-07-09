// Data — FlowScript node declarations (generated, do not edit).
// One declare-function per catalog node. Names are camelCase node types.

// === Data/Atlassian ===

/**
 * Get the current authenticated user's Atlassian account information (cross-product)
 * @param provider — Atlassian provider
 * @returns me — Current user's Atlassian account
 * @returns accountId — The user's globally unique Atlassian account ID
 * @returns email — The user's email address
 * @returns name — The user's display name
 * @impure has side effects / drives control flow
 */
declare function dataAtlassianGetMe({ provider: Struct }): { me: Struct, accountId: string, email: string, name: string };

/**
 * Connect to Jira and Confluence using an API Token. For cloud: create token at id.atlassian.com/manage-profile/security/api-tokens. For server: use personal access token.
 * @param baseUrl (optional) — Your Atlassian instance URL. Cloud: https://your-domain.atlassian.net, Server: https://your-server.com
 * @param email — Your Atlassian account email (required for cloud API tokens, optional for server PAT)
 * @param apiToken — Your API token or Personal Access Token
 * @param isCloud (optional) — Whether this is an Atlassian Cloud instance (affects API version)
 * @returns provider — Atlassian provider for Jira and Confluence APIs
 */
declare function dataAtlassianProviderApiToken({ baseUrl?: string, email: string, apiToken: string, isCloud?: bool }): Struct;

/**
 * Connect to Jira and Confluence using OAuth 2.0. Requires OAuth provider configuration in flow-like.config.json.
 * @param baseUrl (optional) — Your Atlassian Cloud instance URL (e.g., https://your-domain.atlassian.net)
 * @returns provider — Atlassian provider for Jira and Confluence APIs
 */
declare function dataAtlassianProviderOauth({ baseUrl?: string }): Struct;


// === Data/Atlassian/Confluence ===

/**
 * Add a comment to a Confluence page
 * @param provider — Atlassian provider
 * @param pageId — The ID of the page to comment on
 * @param body — The comment content (markdown for cloud, storage format for server)
 * @returns comment — The created comment
 * @impure has side effects / drives control flow
 */
declare function dataAtlassianConfluenceAddComment({ provider: Struct, pageId: string, body: string }): Struct;

/**
 * Add a label to a Confluence page
 * @param provider — Atlassian provider
 * @param pageId — The ID of the page to add the label to
 * @param label — The label name to add
 * @returns success — Whether the label was added successfully
 * @impure has side effects / drives control flow
 */
declare function dataAtlassianConfluenceAddLabel({ provider: Struct, pageId: string, label: string }): bool;

/**
 * Create a new Confluence page
 * @param provider — Atlassian provider (from Atlassian node)
 * @param spaceKey — The space key where the page will be created
 * @param title — Page title
 * @param body (optional) — Page body content (HTML/storage format)
 * @param parentId (optional) — Parent page ID (optional - creates as child page)
 * @returns page — The created Confluence page
 * @returns pageId — The ID of the created page
 * @impure has side effects / drives control flow
 */
declare function dataAtlassianConfluenceCreatePage({ provider: Struct, spaceKey: string, title: string, body?: string, parentId?: string }): { page: Struct, pageId: string };

/**
 * Delete a Confluence attachment
 * @param provider — Atlassian provider
 * @param attachmentId — The attachment content ID to delete
 * @returns success — Whether the attachment was deleted
 * @impure has side effects / drives control flow
 */
declare function dataAtlassianConfluenceDeleteAttachment({ provider: Struct, attachmentId: string }): bool;

/**
 * Delete a Confluence page. Use with caution - this action cannot be undone.
 * @param provider — Atlassian provider
 * @param pageId — The ID of the page to delete
 * @returns success — Whether the deletion was successful
 * @impure has side effects / drives control flow
 */
declare function dataAtlassianConfluenceDeletePage({ provider: Struct, pageId: string }): bool;

/**
 * Download a Confluence attachment to a FlowPath
 * @param provider — Atlassian provider
 * @param attachmentId — The attachment content ID to download
 * @param outputPath — FlowPath to write the downloaded attachment into
 * @returns path — Written file path
 * @returns attachment — Downloaded attachment metadata
 * @returns size — Size in bytes
 * @impure has side effects / drives control flow
 */
declare function dataAtlassianConfluenceDownloadAttachment({ provider: Struct, attachmentId: string, outputPath: Struct }): { path: Struct, attachment: Struct, size: int };

/**
 * Get comments from a Confluence page
 * @param provider — Atlassian provider
 * @param pageId — The ID of the page to get comments from
 * @returns comments — List of comments on the page
 * @impure has side effects / drives control flow
 */
declare function dataAtlassianConfluenceGetComments({ provider: Struct, pageId: string }): Struct[];

/**
 * Get the profile of the currently authenticated user
 * @param provider — Atlassian provider
 * @returns user — Current user profile
 * @impure has side effects / drives control flow
 */
declare function dataAtlassianConfluenceGetCurrentUser({ provider: Struct }): Struct;

/**
 * Get all labels for a Confluence page
 * @param provider — Atlassian provider
 * @param pageId — The ID of the page to get labels for
 * @returns labels — List of labels
 * @returns count — Number of labels
 * @impure has side effects / drives control flow
 */
declare function dataAtlassianConfluenceGetLabels({ provider: Struct, pageId: string }): { labels: Struct[], count: int };

/**
 * Get a Confluence page by its ID
 * @param provider — Atlassian provider (from Atlassian node)
 * @param pageId — The page ID to retrieve
 * @param includeBody (optional) — Whether to include the page body content
 * @param bodyFormat (optional) — Format for the body content
 * @returns page — The Confluence page
 * @returns bodyContent — The page body as plain text/HTML
 * @impure has side effects / drives control flow
 */
declare function dataAtlassianConfluenceGetPage({ provider: Struct, pageId: string, includeBody?: bool, bodyFormat?: string }): { page: Struct, bodyContent: string };

/**
 * Get the ancestor pages (parent hierarchy) of a page
 * @param provider — Atlassian provider
 * @param pageId — The ID of the page to get ancestors for
 * @returns ancestors — List of ancestor pages (from root to immediate parent)
 * @returns depth — Depth in page hierarchy
 * @impure has side effects / drives control flow
 */
declare function dataAtlassianConfluenceGetPageAncestors({ provider: Struct, pageId: string }): { ancestors: Struct[], depth: int };

/**
 * Get all child pages of a Confluence page
 * @param provider — Atlassian provider
 * @param pageId — The ID of the parent page
 * @param expand — Properties to expand (comma-separated, e.g., 'body.storage,version')
 * @param limit — Maximum number of children to return (default: 25)
 * @returns children — List of child pages
 * @returns count — Number of children
 * @impure has side effects / drives control flow
 */
declare function dataAtlassianConfluenceGetPageChildren({ provider: Struct, pageId: string, expand: string, limit: int }): { children: Struct[], count: int };

/**
 * List attachments on a Confluence page
 * @param provider — Atlassian provider
 * @param pageId — The page ID to list attachments for
 * @param limit (optional) — Maximum number of attachments to return
 * @returns attachments — List of attachments
 * @returns count — Number of attachments
 * @impure has side effects / drives control flow
 */
declare function dataAtlassianConfluenceListAttachments({ provider: Struct, pageId: string, limit?: int }): { attachments: Struct[], count: int };

/**
 * List all accessible Confluence spaces
 * @param provider — Atlassian provider (from Atlassian node)
 * @param spaceType (optional) — Filter by space type
 * @param status (optional) — Filter by space status
 * @param limit (optional) — Maximum number of spaces to return (1-100)
 * @param start (optional) — Index of the first result to return (for pagination)
 * @returns spaces — Array of Confluence spaces
 * @returns count — Number of spaces returned
 * @impure has side effects / drives control flow
 */
declare function dataAtlassianConfluenceListSpaces({ provider: Struct, spaceType?: string, status?: string, limit?: int, start?: int }): { spaces: Struct[], count: int };

/**
 * Remove a label from a Confluence page
 * @param provider — Atlassian provider
 * @param pageId — The ID of the page to remove the label from
 * @param label — The label name to remove
 * @returns success — Whether the label was removed successfully
 * @impure has side effects / drives control flow
 */
declare function dataAtlassianConfluenceRemoveLabel({ provider: Struct, pageId: string, label: string }): bool;

/**
 * Search Confluence content using CQL (Confluence Query Language) or text search
 * @param provider — Atlassian provider (from Atlassian node)
 * @param cql (optional) — CQL query string (e.g., 'space = TEAM AND type = page AND text ~ "search term"')
 * @param text (optional) — Simple text search (alternative to CQL)
 * @param spaceKey (optional) — Limit search to a specific space (optional)
 * @param contentType (optional) — Filter by content type
 * @param limit (optional) — Maximum number of results to return (1-100)
 * @param start (optional) — Index of the first result to return (for pagination)
 * @returns results — Array of search results
 * @returns total — Total number of matching results
 * @returns hasMore — Whether there are more results available
 * @impure has side effects / drives control flow
 */
declare function dataAtlassianConfluenceSearch({ provider: Struct, cql?: string, text?: string, spaceKey?: string, contentType?: string, limit?: int, start?: int }): { results: Struct[], total: int, hasMore: bool };

/**
 * Search for users in Confluence
 * @param provider — Atlassian provider
 * @param query — Search query for user name or email
 * @param limit — Maximum number of users to return (default: 25)
 * @returns users — List of matching users
 * @returns count — Number of users found
 * @impure has side effects / drives control flow
 */
declare function dataAtlassianConfluenceSearchUsers({ provider: Struct, query: string, limit: int }): { users: Struct[], count: int };

/**
 * Update an existing Confluence page's title or body
 * @param provider — Atlassian provider (from Atlassian node)
 * @param pageId — The ID of the page to update
 * @param title (optional) — New page title (leave empty to keep current)
 * @param body (optional) — New page body content (HTML/storage format, leave empty to keep current)
 * @param versionMessage (optional) — Optional message for this version (shows in page history)
 * @returns page — The updated Confluence page
 * @impure has side effects / drives control flow
 */
declare function dataAtlassianConfluenceUpdatePage({ provider: Struct, pageId: string, title?: string, body?: string, versionMessage?: string }): Struct;

/**
 * Upload a file attachment to a Confluence page
 * @param provider — Atlassian provider
 * @param pageId — The page ID to upload the attachment to
 * @param file — File to upload
 * @param filename (optional) — Override file name for the uploaded attachment (optional)
 * @param comment (optional) — Attachment version comment (optional)
 * @returns attachments — Created or updated attachments
 * @returns count — Number of attachments
 * @impure has side effects / drives control flow
 */
declare function dataAtlassianConfluenceUploadAttachment({ provider: Struct, pageId: string, file: Struct, filename?: string, comment?: string }): { attachments: Struct[], count: int };


// === Data/Atlassian/Jira ===

/**
 * Add a comment to a Jira issue
 * @param provider — Atlassian provider
 * @param issueKey — The issue key (e.g., PROJ-123)
 * @param body — The comment text (supports markdown for cloud, wiki markup for server)
 * @returns comment — The created comment
 * @impure has side effects / drives control flow
 */
declare function dataAtlassianJiraAddComment({ provider: Struct, issueKey: string, body: string }): Struct;

/**
 * Add a work log entry to a Jira issue
 * @param provider — Atlassian provider
 * @param issueKey — The issue key (e.g., PROJ-123)
 * @param timeSpent — Time spent in Jira format (e.g., '2h 30m', '1d', '30m')
 * @param comment — Optional comment for the work log
 * @param started — When the work was started (ISO 8601 format, defaults to now)
 * @returns worklog — The created work log entry
 * @impure has side effects / drives control flow
 */
declare function dataAtlassianJiraAddWorklog({ provider: Struct, issueKey: string, timeSpent: string, comment: string, started: string }): Struct;

/**
 * Create multiple Jira issues in a batch
 * @param provider — Atlassian provider
 * @param issues — Array of issues to create
 * @returns results — Results for each issue creation
 * @returns createdCount — Number of successfully created issues
 * @returns failedCount — Number of failed issue creations
 * @impure has side effects / drives control flow
 */
declare function dataAtlassianJiraBatchCreateIssues({ provider: Struct, issues: Struct[] }): { results: Struct[], createdCount: int, failedCount: int };

/**
 * Create multiple versions (releases) in a batch
 * @param provider — Atlassian provider
 * @param versions — Array of versions to create
 * @returns results — Results for each version creation
 * @returns createdCount — Number of successfully created versions
 * @returns failedCount — Number of failed version creations
 * @impure has side effects / drives control flow
 */
declare function dataAtlassianJiraBatchCreateVersions({ provider: Struct, versions: Struct[] }): { results: Struct[], createdCount: int, failedCount: int };

/**
 * Get changelogs for multiple issues at once
 * @param provider — Atlassian provider
 * @param issueKeys (optional) — Issue keys to fetch changelogs for
 * @returns results — Changelog entries grouped by issue key
 * @impure has side effects / drives control flow
 */
declare function dataAtlassianJiraBatchGetChangelogs({ provider: Struct, issueKeys?: string[] }): Struct[];

/**
 * Create a new Jira issue
 * @param provider — Atlassian provider (from Atlassian node)
 * @param projectKey — The project key (e.g., PROJ)
 * @param issueType (optional) — The issue type name (e.g., Bug, Story, Task)
 * @param summary — Issue summary/title
 * @param description (optional) — Issue description (plain text)
 * @param priority (optional) — Issue priority name (e.g., Highest, High, Medium, Low, Lowest)
 * @param assigneeId (optional) — Account ID of the assignee (leave empty for unassigned)
 * @param labels (optional) — Labels to assign to the issue
 * @param parentKey (optional) — Parent issue key for subtasks (e.g., PROJ-123)
 * @returns issue — The created Jira issue
 * @returns issueKey — The key of the created issue (e.g., PROJ-123)
 * @impure has side effects / drives control flow
 */
declare function dataAtlassianJiraCreateIssue({ provider: Struct, projectKey: string, issueType?: string, summary: string, description?: string, priority?: string, assigneeId?: string, labels?: string[], parentKey?: string }): { issue: Struct, issueKey: string };

/**
 * Create a link between two issues
 * @param provider — Atlassian provider
 * @param linkType — The name of the link type (e.g., 'Blocks', 'Cloners', 'Relates')
 * @param inwardIssue — The inward issue key (e.g., PROJ-123)
 * @param outwardIssue — The outward issue key (e.g., PROJ-456)
 * @param comment — Optional comment for the link
 * @returns success — Whether the link was created successfully
 * @impure has side effects / drives control flow
 */
declare function dataAtlassianJiraCreateIssueLink({ provider: Struct, linkType: string, inwardIssue: string, outwardIssue: string, comment: string }): bool;

/**
 * Create a new version (release) in a project
 * @param provider — Atlassian provider
 * @param name — Name of the version
 * @param projectKey — The project key (e.g., PROJ)
 * @param description — Description of the version (optional)
 * @param releaseDate — Planned release date (YYYY-MM-DD, optional)
 * @param startDate — Start date (YYYY-MM-DD, optional)
 * @param released — Whether the version is already released (default: false)
 * @returns version — The created version
 * @impure has side effects / drives control flow
 */
declare function dataAtlassianJiraCreateVersion({ provider: Struct, name: string, projectKey: string, description: string, releaseDate: string, startDate: string, released: bool }): Struct;

/**
 * Delete an attachment from an issue
 * @param provider — Atlassian provider
 * @param attachmentId — The ID of the attachment to delete
 * @returns success — Whether the deletion was successful
 * @impure has side effects / drives control flow
 */
declare function dataAtlassianJiraDeleteAttachment({ provider: Struct, attachmentId: string }): bool;

/**
 * Delete a Jira issue. Use with caution - this action cannot be undone.
 * @param provider — Atlassian provider
 * @param issueKey — The issue key to delete (e.g., PROJ-123)
 * @param deleteSubtasks — Also delete subtasks (required if issue has subtasks)
 * @returns success — Whether the deletion was successful
 * @impure has side effects / drives control flow
 */
declare function dataAtlassianJiraDeleteIssue({ provider: Struct, issueKey: string, deleteSubtasks: bool }): bool;

/**
 * Download the content of an attachment
 * @param provider — Atlassian provider
 * @param attachmentId — The ID of the attachment to download
 * @param outputPath — FlowPath to write the downloaded attachment into
 * @returns path — Written file path
 * @returns size — Size of the downloaded content in bytes
 * @impure has side effects / drives control flow
 */
declare function dataAtlassianJiraDownloadAttachment({ provider: Struct, attachmentId: string, outputPath: Struct }): { path: Struct, size: int };

/**
 * Get all attachments for a Jira issue
 * @param provider — Atlassian provider
 * @param issueKey — The issue key (e.g., PROJ-123)
 * @returns attachments — List of attachments
 * @returns count — Number of attachments
 * @impure has side effects / drives control flow
 */
declare function dataAtlassianJiraGetAttachments({ provider: Struct, issueKey: string }): { attachments: Struct[], count: int };

/**
 * Get the change history for an issue
 * @param provider — Atlassian provider
 * @param issueKey — The issue key (e.g., PROJ-123)
 * @param maxResults — Maximum number of changelog entries (default: 100)
 * @returns changelog — List of changelog entries
 * @returns total — Total number of changelog entries
 * @impure has side effects / drives control flow
 */
declare function dataAtlassianJiraGetChangelog({ provider: Struct, issueKey: string, maxResults: int }): { changelog: Struct[], total: int };

/**
 * Get the profile of the currently authenticated user
 * @param provider — Atlassian provider
 * @returns user — Current user profile
 * @impure has side effects / drives control flow
 */
declare function dataAtlassianJiraGetCurrentUser({ provider: Struct }): Struct;

/**
 * Get all issues linked to an epic
 * @param provider — Atlassian provider
 * @param epicKey — The epic key (e.g., PROJ-100)
 * @param maxResults — Maximum number of issues to return (default: 50)
 * @returns issues — Issues linked to the epic
 * @returns count — Number of issues found
 * @impure has side effects / drives control flow
 */
declare function dataAtlassianJiraGetEpicIssues({ provider: Struct, epicKey: string, maxResults: int }): { issues: Struct[], count: int };

/**
 * Get all available fields in Jira (system and custom fields)
 * @param provider — Atlassian provider
 * @returns fields — All available fields
 * @returns systemFields — System fields only
 * @returns customFields — Custom fields only
 * @impure has side effects / drives control flow
 */
declare function dataAtlassianJiraGetFields({ provider: Struct }): { fields: Struct[], systemFields: Struct[], customFields: Struct[] };

/**
 * Get a single Jira issue by its key (e.g., PROJ-123)
 * @param provider — Atlassian provider (from Atlassian node)
 * @param issueKey — The issue key (e.g., PROJ-123) or ID
 * @param includeComments (optional) — Whether to fetch comments for the issue
 * @returns issue — The Jira issue
 * @returns comments — Comments on the issue (if requested)
 * @impure has side effects / drives control flow
 */
declare function dataAtlassianJiraGetIssue({ provider: Struct, issueKey: string, includeComments?: bool }): { issue: Struct, comments: Struct[] };

/**
 * Get all links for an issue
 * @param provider — Atlassian provider
 * @param issueKey — The issue key (e.g., PROJ-123)
 * @returns links — Issue links
 * @returns count — Number of links
 * @impure has side effects / drives control flow
 */
declare function dataAtlassianJiraGetIssueLinks({ provider: Struct, issueKey: string }): { links: Struct[], count: int };

/**
 * Get all available issue link types
 * @param provider — Atlassian provider
 * @returns linkTypes — Available link types
 * @impure has side effects / drives control flow
 */
declare function dataAtlassianJiraGetLinkTypes({ provider: Struct }): Struct[];

/**
 * Get all issues for a specific Jira project
 * @param provider — Atlassian provider
 * @param projectKey — The project key (e.g., PROJ)
 * @param jqlFilter — Additional JQL filter to apply (optional, will be combined with project filter)
 * @param maxResults (optional) — Maximum number of issues to return (default 50, max 100)
 * @param startAt (optional) — Index to start at for server/Data Center pagination
 * @param nextPageToken (optional) — Cloud pagination token from a previous response
 * @returns issues — List of issues in the project
 * @returns total — Total number of issues
 * @returns count — Number of issues returned in this response
 * @returns nextPageTokenOut — Cloud pagination token for the next page
 * @impure has side effects / drives control flow
 */
declare function dataAtlassianJiraGetProjectIssues({ provider: Struct, projectKey: string, jqlFilter: string, maxResults?: int, startAt?: int, nextPageToken?: string }): { issues: Struct[], total: int, count: int, nextPageTokenOut: string };

/**
 * Get available workflow transitions for a Jira issue
 * @param provider — Atlassian provider
 * @param issueKey — The issue key (e.g., PROJ-123)
 * @returns transitions — Available transitions for the issue
 * @impure has side effects / drives control flow
 */
declare function dataAtlassianJiraGetTransitions({ provider: Struct, issueKey: string }): Struct[];

/**
 * Get all versions (releases) for a project
 * @param provider — Atlassian provider
 * @param projectKey — The project key (e.g., PROJ)
 * @returns versions — List of versions
 * @returns count — Number of versions
 * @impure has side effects / drives control flow
 */
declare function dataAtlassianJiraGetVersions({ provider: Struct, projectKey: string }): { versions: Struct[], count: int };

/**
 * Get work log entries for a Jira issue
 * @param provider — Atlassian provider
 * @param issueKey — The issue key (e.g., PROJ-123)
 * @returns worklogs — List of work log entries
 * @returns totalTimeSpent — Total time spent in seconds
 * @impure has side effects / drives control flow
 */
declare function dataAtlassianJiraGetWorklog({ provider: Struct, issueKey: string }): { worklogs: Struct[], totalTimeSpent: int };

/**
 * Link an issue to an epic (adds issue to epic's child issues)
 * @param provider — Atlassian provider
 * @param issueKey — The issue key to link to the epic (e.g., PROJ-123)
 * @param epicKey — The epic key to link the issue to (e.g., PROJ-100)
 * @returns success — Whether the linking was successful
 * @impure has side effects / drives control flow
 */
declare function dataAtlassianJiraLinkToEpic({ provider: Struct, issueKey: string, epicKey: string }): bool;

/**
 * List all accessible Jira projects
 * @param provider — Atlassian provider (from Atlassian node)
 * @param maxResults (optional) — Maximum number of projects to return (1-100)
 * @param startAt (optional) — Index of the first result to return (for pagination)
 * @param query (optional) — Filter projects by name or key (partial match)
 * @returns projects — Array of Jira projects
 * @returns count — Number of projects returned
 * @impure has side effects / drives control flow
 */
declare function dataAtlassianJiraListProjects({ provider: Struct, maxResults?: int, startAt?: int, query?: string }): { projects: Struct[], count: int };

/**
 * Remove a link between issues
 * @param provider — Atlassian provider
 * @param linkId — The ID of the link to remove
 * @returns success — Whether the link was removed successfully
 * @impure has side effects / drives control flow
 */
declare function dataAtlassianJiraRemoveIssueLink({ provider: Struct, linkId: string }): bool;

/**
 * Search for Jira fields by name, type, or key
 * @param provider — Atlassian provider
 * @param query — Search query for field name or key
 * @param onlyCustom — Only return custom fields
 * @param schemaType — Filter by schema type (e.g., 'string', 'array', 'option', 'user')
 * @returns fields — Matching fields
 * @returns count — Number of matching fields
 * @impure has side effects / drives control flow
 */
declare function dataAtlassianJiraSearchFields({ provider: Struct, query: string, onlyCustom: bool, schemaType: string }): { fields: Struct[], count: int };

/**
 * Search for Jira issues using JQL (Jira Query Language)
 * @param provider — Atlassian provider (from Atlassian node)
 * @param jql (optional) — JQL query string (e.g., 'project = PROJ AND status = "In Progress"')
 * @param maxResults (optional) — Maximum number of results to return (1-100)
 * @param startAt (optional) — Index of the first result to return (server/Data Center pagination)
 * @param fields (optional) — Fields to return (leave empty for default fields)
 * @param nextPageToken (optional) — Cloud pagination token from a previous search response
 * @returns issues — Array of Jira issues matching the query
 * @returns total — Total number of matching issues
 * @returns hasMore — Whether there are more results available
 * @returns nextPageTokenOut — Cloud pagination token for the next page
 * @impure has side effects / drives control flow
 */
declare function dataAtlassianJiraSearchIssues({ provider: Struct, jql?: string, maxResults?: int, startAt?: int, fields?: string[], nextPageToken?: string }): { issues: Struct[], total: int, hasMore: bool, nextPageTokenOut: string };

/**
 * Change the status of a Jira issue by applying a transition
 * @param provider — Atlassian provider (from Atlassian node)
 * @param issueKey — The issue key (e.g., PROJ-123)
 * @param transitionId (optional) — The ID of the transition to apply (use 'List Transitions' to get available IDs)
 * @param transitionName (optional) — The name of the transition (alternative to ID, e.g., 'Done', 'In Progress')
 * @param comment (optional) — Add a comment while transitioning (optional)
 * @returns issue — The issue after transition
 * @returns availableTransitions — List of available transitions for the issue (populated if transition fails or for reference)
 * @impure has side effects / drives control flow
 */
declare function dataAtlassianJiraTransitionIssue({ provider: Struct, issueKey: string, transitionId?: string, transitionName?: string, comment?: string }): { issue: Struct, availableTransitions: Struct[] };

/**
 * Remove an issue from its epic
 * @param provider — Atlassian provider
 * @param issueKey — The issue key to unlink from its epic (e.g., PROJ-123)
 * @returns success — Whether the unlinking was successful
 * @impure has side effects / drives control flow
 */
declare function dataAtlassianJiraUnlinkFromEpic({ provider: Struct, issueKey: string }): bool;

/**
 * Update an existing Jira issue's fields
 * @param provider — Atlassian provider (from Atlassian node)
 * @param issueKey — The issue key (e.g., PROJ-123) to update
 * @param summary (optional) — New summary/title (leave empty to keep current)
 * @param description (optional) — New description (leave empty to keep current)
 * @param priority (optional) — New priority name (leave empty to keep current)
 * @param assigneeId (optional) — New assignee account ID (leave empty to keep current, use 'unassigned' to remove)
 * @param labels (optional) — New labels (replaces existing labels, leave empty to keep current)
 * @param comment (optional) — Add a comment while updating (optional)
 * @returns issue — The updated Jira issue
 * @impure has side effects / drives control flow
 */
declare function dataAtlassianJiraUpdateIssue({ provider: Struct, issueKey: string, summary?: string, description?: string, priority?: string, assigneeId?: string, labels?: string[], comment?: string }): Struct;

/**
 * Update an existing version
 * @param provider — Atlassian provider
 * @param versionId — The version ID to update
 * @param name — New name for the version (optional)
 * @param description — New description (optional)
 * @param released — Set released status (optional)
 * @param archived — Set archived status (optional)
 * @param releaseDate — New release date (YYYY-MM-DD, optional)
 * @returns version — The updated version
 * @impure has side effects / drives control flow
 */
declare function dataAtlassianJiraUpdateVersion({ provider: Struct, versionId: string, name: string, description: string, released: bool, archived: bool, releaseDate: string }): Struct;

/**
 * Upload a file attachment to a Jira issue
 * @param provider — Atlassian provider
 * @param issueKey — The issue key (e.g., PROJ-123)
 * @param file — File to upload
 * @param filename (optional) — Override file name for the uploaded attachment (optional)
 * @returns attachments — Created Jira attachments
 * @returns count — Number of attachments created
 * @impure has side effects / drives control flow
 */
declare function dataAtlassianJiraUploadAttachment({ provider: Struct, issueKey: string, file: Struct, filename?: string }): { attachments: Struct[], count: int };


// === Data/Atlassian/Jira/Agile ===

/**
 * Create a new sprint on a board
 * @param provider — Atlassian provider
 * @param name — Name of the sprint
 * @param boardId — The board ID to create the sprint on
 * @param goal — Sprint goal (optional)
 * @param startDate — Sprint start date (ISO 8601, optional)
 * @param endDate — Sprint end date (ISO 8601, optional)
 * @returns sprint — The created sprint
 * @impure has side effects / drives control flow
 */
declare function dataAtlassianJiraCreateSprint({ provider: Struct, name: string, boardId: int, goal: string, startDate: string, endDate: string }): Struct;

/**
 * Get backlog issues for a board
 * @param provider — Atlassian provider
 * @param boardId — The board ID to get backlog from
 * @param maxResults — Maximum number of results (default: 50)
 * @returns issues — Backlog issues
 * @returns total — Total backlog items
 * @impure has side effects / drives control flow
 */
declare function dataAtlassianJiraGetBacklog({ provider: Struct, boardId: int, maxResults: int }): { issues: Struct[], total: int };

/**
 * Get all issues on an agile board
 * @param provider — Atlassian provider
 * @param boardId — The board ID to get issues from
 * @param jql — Additional JQL filter (optional)
 * @param maxResults — Maximum number of results (default: 50)
 * @param startAt — Index to start at for pagination (default: 0)
 * @returns issues — List of issues on the board
 * @returns total — Total number of issues
 * @impure has side effects / drives control flow
 */
declare function dataAtlassianJiraGetBoardIssues({ provider: Struct, boardId: int, jql: string, maxResults: int, startAt: int }): { issues: Struct[], total: int };

/**
 * Get all agile boards (Scrum or Kanban)
 * @param provider — Atlassian provider
 * @param projectKey — Filter boards by project key (optional)
 * @param boardType — Filter by board type: 'scrum' or 'kanban' (optional)
 * @param name — Filter boards by name (partial match, optional)
 * @returns boards — List of boards
 * @returns count — Number of boards
 * @impure has side effects / drives control flow
 */
declare function dataAtlassianJiraGetBoards({ provider: Struct, projectKey: string, boardType: string, name: string }): { boards: Struct[], count: int };

/**
 * Get all issues in a sprint
 * @param provider — Atlassian provider
 * @param sprintId — The sprint ID to get issues from
 * @param jql — Additional JQL filter (optional)
 * @param maxResults — Maximum number of results (default: 50)
 * @returns issues — Issues in the sprint
 * @returns total — Total issues in sprint
 * @impure has side effects / drives control flow
 */
declare function dataAtlassianJiraGetSprintIssues({ provider: Struct, sprintId: int, jql: string, maxResults: int }): { issues: Struct[], total: int };

/**
 * Get all sprints for a board
 * @param provider — Atlassian provider
 * @param boardId — The board ID to get sprints from
 * @param state — Filter by sprint state: 'active', 'closed', 'future' (optional, comma-separated for multiple)
 * @returns sprints — List of sprints
 * @returns count — Number of sprints
 * @impure has side effects / drives control flow
 */
declare function dataAtlassianJiraGetSprints({ provider: Struct, boardId: int, state: string }): { sprints: Struct[], count: int };

/**
 * Move issues to a sprint
 * @param provider — Atlassian provider
 * @param sprintId — The sprint ID to move issues to
 * @param issueKeys (optional) — Issue keys to move
 * @returns success — Whether the move was successful
 * @impure has side effects / drives control flow
 */
declare function dataAtlassianJiraMoveToSprint({ provider: Struct, sprintId: int, issueKeys?: string[] }): bool;

/**
 * Update an existing sprint
 * @param provider — Atlassian provider
 * @param sprintId — The sprint ID to update
 * @param name — New name for the sprint (optional)
 * @param goal — New sprint goal (optional)
 * @param state — New state: 'active', 'closed', 'future' (optional)
 * @param startDate — New start date (ISO 8601, optional)
 * @param endDate — New end date (ISO 8601, optional)
 * @returns sprint — The updated sprint
 * @impure has side effects / drives control flow
 */
declare function dataAtlassianJiraUpdateSprint({ provider: Struct, sprintId: int, name: string, goal: string, state: string, startDate: string, endDate: string }): Struct;


// === Data/DataFusion ===

/**
 * Creates a new DataFusion session for SQL analytics. Configure optimization settings for production workloads.
 * @param sessionName (optional) — Unique name for this session (used for caching)
 * @param targetPartitions (optional) — Number of partitions for parallel query execution. Higher values increase parallelism but add overhead. 0 = auto (uses CPU count).
 * @param batchSize (optional) — Number of rows processed per batch. Larger batches improve throughput but use more memory.
 * @param repartitionJoins (optional) — Enable automatic repartitioning before joins for better parallelism
 * @param repartitionAggregations (optional) — Enable automatic repartitioning before aggregations
 * @param repartitionSorts (optional) — Enable automatic repartitioning for parallel sorting
 * @param coalesceBatches (optional) — Combine small batches into larger ones to reduce overhead
 * @param parquetPruning (optional) — Enable predicate pushdown and column pruning for Parquet files
 * @param collectStatistics (optional) — Collect statistics from data sources for query optimization
 * @returns session — DataFusion session reference for use with other DataFusion nodes
 * @impure has side effects / drives control flow
 */
declare function dfCreateSession({ sessionName?: string, targetPartitions?: int, batchSize?: int, repartitionJoins?: bool, repartitionAggregations?: bool, repartitionSorts?: bool, coalesceBatches?: bool, parquetPruning?: bool, collectStatistics?: bool }): Struct;

/**
 * Mount CSV files from a FlowPath into a DataFusion session as a queryable table
 * @param session — DataFusion session to mount the table into
 * @param path — FlowPath to CSV files (can be a directory prefix or single file)
 * @param tableName — Name to register the table as in the DataFusion catalog
 * @param hasHeader (optional) — Whether the CSV files have a header row
 * @param delimiter (optional) — Column delimiter character
 * @param fileExtension (optional) — File extension filter
 * @impure has side effects / drives control flow
 */
declare function dfMountCsv({ session: Struct, path: Struct, tableName: string, hasHeader?: bool, delimiter?: string, fileExtension?: string }): void;

/**
 * Mount JSON (newline-delimited) files from a FlowPath into a DataFusion session as a queryable table
 * @param session — DataFusion session to mount the table into
 * @param path — FlowPath to JSON files (can be a directory prefix or single file)
 * @param tableName — Name to register the table as in the DataFusion catalog
 * @param fileExtension (optional) — File extension filter
 * @impure has side effects / drives control flow
 */
declare function dfMountJson({ session: Struct, path: Struct, tableName: string, fileExtension?: string }): void;

/**
 * Mount Parquet files from a FlowPath prefix into a DataFusion session as a queryable table
 * @param session — DataFusion session to mount the table into
 * @param path — FlowPath to Parquet files (can be a directory prefix or single file)
 * @param tableName — Name to register the table as in the DataFusion catalog
 * @param fileExtension (optional) — File extension filter (e.g., 'parquet', 'parquet.gz')
 * @impure has side effects / drives control flow
 */
declare function dfMountParquet({ session: Struct, path: Struct, tableName: string, fileExtension?: string }): void;

/**
 * Register a CSVTable (from Excel/CSV extraction) into a DataFusion session for SQL queries. Converts the table to an in-memory Arrow table.
 * @param session — DataFusion session to register the table into
 * @param table — CSVTable to register (from Excel/CSV extraction nodes)
 * @param tableName (optional) — Name to register the table as in the DataFusion catalog
 * @impure has side effects / drives control flow
 */
declare function dfRegisterCsvTable({ session: Struct, table: Struct, tableName?: string }): void;

/**
 * Register a LanceDB table into a DataFusion session for SQL queries. Uses the existing to_datafusion() implementation from the vector store.
 * @param session — DataFusion session to register the table into
 * @param database — LanceDB database connection
 * @param tableName (optional) — Name to register the table as in the DataFusion catalog. If empty, uses the database's original table name.
 * @impure has side effects / drives control flow
 */
declare function dfRegisterLance({ session: Struct, database: Struct, tableName?: string }): void;

/**
 * Execute a SQL query against a DataFusion session. Returns results as both a CSVTable (for analytics) and array of row objects (for iteration).
 * @param session — DataFusion session with registered tables
 * @param query (optional) — SQL query to execute (e.g., SELECT * FROM mytable WHERE column > 10)
 * @returns table — Query results as a CSVTable (columnar format, good for analytics)
 * @returns rows — Query results as array of row structs with Flow-Like-compatible values
 * @returns rowCount — Number of rows in the result
 * @impure has side effects / drives control flow
 */
declare function dfSqlQuery({ session: Struct, query?: string }): { table: Struct, rows: Struct[], rowCount: int };


// === Data/DataFusion/Aggregation ===

/**
 * Truncate timestamps to a specific precision (hour, day, month, etc.) and aggregate. Simpler alternative to date_bin for standard intervals.
 * @param session — DataFusion session
 * @param sourceTable — Table to aggregate
 * @param timestampColumn — Timestamp column name
 * @param precision (optional) — Truncation precision: second, minute, hour, day, week, month, quarter, year
 * @param aggregationSql — SQL aggregation expressions (e.g., 'COUNT(*) as cnt, SUM(amount) as total')
 * @param filter (optional) — Optional WHERE clause
 * @returns sessionOut — Session pass-through
 * @returns results — Aggregation results
 * @returns sql — Generated SQL
 * @impure has side effects / drives control flow
 */
declare function dfDateTruncAggregation({ session: Struct, sourceTable: string, timestampColumn: string, precision?: string, aggregationSql: string, filter?: string }): { sessionOut: Struct, results: Struct[], sql: string };

/**
 * Create time-based aggregations using DataFusion's date_bin function. Groups data by fixed time intervals (minute, hour, day, etc.) and applies aggregation functions.
 * @param session — DataFusion session to execute the query in
 * @param sourceTable — Name of the table to aggregate
 * @param timestampColumn — Column containing timestamp/datetime values
 * @param interval (optional) — Time interval for binning: second, minute, 5m, 15m, 30m, hour, day, week, month, quarter, year
 * @param valueColumns — Columns to aggregate (comma-separated)
 * @param aggregations (optional) — Aggregation functions to apply (comma-separated): count, sum, avg, min, max, first, last
 * @param groupBy (optional) — Additional columns to group by (comma-separated, optional)
 * @param filter (optional) — Optional WHERE clause filter (e.g., 'status = active')
 * @param outputTable (optional) — Name for the result table (optional, creates view if provided)
 * @returns sessionOut — DataFusion session (pass-through)
 * @returns results — Query results as array of row structs
 * @returns sql — Generated SQL query for debugging
 * @returns rowCount — Number of result rows
 * @impure has side effects / drives control flow
 */
declare function dfTimeBinAggregation({ session: Struct, sourceTable: string, timestampColumn: string, interval?: string, valueColumns: string, aggregations?: string, groupBy?: string, filter?: string, outputTable?: string }): { sessionOut: Struct, results: Struct[], sql: string, rowCount: int };

/**
 * Apply window functions for rolling/moving aggregations over time series data.
 * @param session — DataFusion session
 * @param sourceTable — Table to query
 * @param timestampColumn — Column for ordering
 * @param valueColumn — Column to aggregate
 * @param windowFunction (optional) — Function: avg, sum, min, max, count, row_number, rank, lag, lead
 * @param windowSize (optional) — Number of preceding rows (for rolling window), use 0 for cumulative
 * @param partitionBy (optional) — Columns to partition by (comma-separated, optional)
 * @param selectColumns (optional) — Additional columns to include (comma-separated)
 * @returns sessionOut — Session pass-through
 * @returns results — Query results
 * @returns sql — Generated SQL
 * @impure has side effects / drives control flow
 */
declare function dfWindowAggregation({ session: Struct, sourceTable: string, timestampColumn: string, valueColumn: string, windowFunction?: string, windowSize?: int, partitionBy?: string, selectColumns?: string }): { sessionOut: Struct, results: Struct[], sql: string };


// === Data/DataFusion/Databases ===

/**
 * Mount Parquet files from an Athena query result location in S3. Supports explicit credentials or environment variables (including Lambda IAM roles).
 * @param session — DataFusion session to register the table in
 * @param s3Path — S3 path to Athena query results (e.g., s3://bucket/athena-results/query-id/)
 * @param region (optional) — AWS region
 * @param credentialMode (optional) — How to authenticate: 'explicit' (access keys), 'environment' (env vars/Lambda IAM role/profile via AWS_PROFILE env var)
 * @param accessKeyId — AWS access key ID (only used when credential_mode is 'explicit')
 * @param secretAccessKey — AWS secret access key (only used when credential_mode is 'explicit')
 * @param sessionToken — Optional AWS session token for temporary credentials
 * @param tableName — Name to register the table as in DataFusion
 * @param format (optional) — File format (parquet, csv)
 * @returns sessionOut — DataFusion session with registered table
 * @impure has side effects / drives control flow
 */
declare function dfMountAthenaQuery({ session: Struct, s3Path: string, region?: string, credentialMode?: string, accessKeyId: string, secretAccessKey: string, sessionToken: string, tableName: string, format?: string }): Struct;

/**
 * Register an AWS Athena table in DataFusion. Query S3 data via Athena's catalog. Supports explicit credentials or environment variables (including Lambda IAM roles).
 * @param session — DataFusion session to register the table in
 * @param region (optional) — AWS region where Athena is configured
 * @param credentialMode (optional) — How to authenticate: 'explicit' (access keys), 'environment' (env vars/Lambda IAM role/profile via AWS_PROFILE env var)
 * @param accessKeyId — AWS access key ID (only used when credential_mode is 'explicit')
 * @param secretAccessKey — AWS secret access key (only used when credential_mode is 'explicit')
 * @param sessionToken — Optional AWS session token for temporary credentials
 * @param catalog (optional) — Athena data catalog name (default: AwsDataCatalog)
 * @param athenaDatabase — Database name in Athena
 * @param athenaTable — Table name in Athena to query
 * @param outputLocation — S3 location for query results (e.g., s3://my-bucket/athena-results/)
 * @param tableName — Name to register the table as in DataFusion
 * @returns sessionOut — DataFusion session with registered table
 * @impure has side effects / drives control flow
 */
declare function dfRegisterAthena({ session: Struct, region?: string, credentialMode?: string, accessKeyId: string, secretAccessKey: string, sessionToken: string, catalog?: string, athenaDatabase: string, athenaTable: string, outputLocation: string, tableName: string }): Struct;

/**
 * Register a Google BigQuery table or query result into a DataFusion session. Takes a GcpProvider for authentication — pair it with the GCP Provider node.
 * @param session — DataFusion session to register the table into
 * @param provider — GCP provider with authentication (from the GCP Provider node)
 * @param projectId (optional) — GCP project ID for billing/job routing. Falls back to the provider's default_project_id when empty.
 * @param registrationMode (optional) — How to select the data: 'table' (register a full BigQuery table) or 'query' (register the result of a Standard SQL query)
 * @param dataset (optional) — BigQuery dataset (only used when registration_mode is 'table')
 * @param sourceTable (optional) — BigQuery table name (only used when registration_mode is 'table')
 * @param rowLimit (optional) — Optional LIMIT applied in 'table' mode. 0 means no limit.
 * @param tableName — Name to register the result as in DataFusion
 * @param location (optional) — BigQuery location for the job (e.g. 'US', 'EU', 'europe-west1')
 * @param pageSize (optional) — Max rows per page when paginating results. 0 lets BigQuery pick (10 MB cap).
 * @param useQueryCache (optional) — Allow BigQuery to serve the result from its query cache when available
 * @param maxBytesBilled (optional) — Cap on bytes billed for this query. 0 means use project default.
 * @returns sessionOut — DataFusion session
 * @returns registeredAs — Final table name registered in the DataFusion session
 * @returns rowCount — Number of rows materialised into the DataFusion session
 * @returns jobStats — BigQuery job statistics (job id, bytes processed, cache hit)
 * @impure has side effects / drives control flow
 */
declare function dfRegisterBigquery({ session: Struct, provider: Struct, projectId?: string, registrationMode?: string, dataset?: string, sourceTable?: string, rowLimit?: int, tableName: string, location?: string, pageSize?: int, useQueryCache?: bool, maxBytesBilled?: int }): { sessionOut: Struct, registeredAs: string, rowCount: int, jobStats: Struct };

/**
 * Register a ClickHouse table for federated queries. Uses real database connection for full SQL push-down.
 * @param session — DataFusion session
 * @param host (optional) — ClickHouse server host
 * @param port (optional) — ClickHouse HTTP port
 * @param database (optional) — Database name
 * @param username (optional) — Database username
 * @param password (optional) — Database password
 * @param sourceTable — Name of the table in ClickHouse
 * @param tableName — Name to register in DataFusion
 * @param readonly (optional) — Use read-only queries
 * @returns sessionOut — DataFusion session
 * @returns connectionUrl — Generated connection URL
 * @impure has side effects / drives control flow
 */
declare function dfRegisterClickhouse({ session: Struct, host?: string, port?: int, database?: string, username?: string, password?: string, sourceTable: string, tableName: string, readonly?: bool }): { sessionOut: Struct, connectionUrl: string };

/**
 * Register a DuckDB database table for federated queries. Uses real database connection.
 * @param session — DataFusion session
 * @param filePath (optional) — Path to DuckDB database file (or :memory:)
 * @param sourceTable — Name of the table in DuckDB
 * @param tableName — Name to register in DataFusion
 * @param readonly (optional) — Open database in read-only mode
 * @returns sessionOut — DataFusion session
 * @impure has side effects / drives control flow
 */
declare function dfRegisterDuckdb({ session: Struct, filePath?: string, sourceTable: string, tableName: string, readonly?: bool }): Struct;

/**
 * Register a table via Arrow Flight SQL protocol. High-performance columnar data transfer (10-100x faster than JDBC/ODBC). Supports Dremio, InfluxDB, DuckDB Flight, ClickHouse Flight, and more.
 * @param session — DataFusion session
 * @param host (optional) — Flight SQL server host
 * @param port (optional) — Flight SQL server port (typically 443 for TLS, or service-specific)
 * @param username (optional) — Username for authentication (optional)
 * @param password (optional) — Password or bearer token for authentication (optional)
 * @param query — SQL query to execute (e.g., SELECT * FROM my_table)
 * @param tableName — Name to register the query result in DataFusion
 * @param useTls (optional) — Enable TLS/SSL encryption for the connection
 * @param skipVerify (optional) — Skip TLS certificate verification (for self-signed certs)
 * @returns sessionOut — DataFusion session
 * @returns endpoint — Flight SQL endpoint URL
 * @impure has side effects / drives control flow
 */
declare function dfRegisterFlightsql({ session: Struct, host?: string, port?: int, username?: string, password?: string, query: string, tableName: string, useTls?: bool, skipVerify?: bool }): { sessionOut: Struct, endpoint: string };

/**
 * Register a MySQL table for federated queries. Uses real database connection for full SQL push-down.
 * @param session — DataFusion session
 * @param host (optional) — MySQL server host
 * @param port (optional) — MySQL server port
 * @param database — Database name
 * @param username — Database username
 * @param password — Database password
 * @param sourceTable — Name of the table in MySQL
 * @param tableName — Name to register in DataFusion
 * @param sslMode (optional) — SSL mode: disabled, preferred, required
 * @param readonly (optional) — Open connection in read-only mode
 * @returns sessionOut — DataFusion session
 * @returns connectionUrl — Generated connection URL
 * @impure has side effects / drives control flow
 */
declare function dfRegisterMysql({ session: Struct, host?: string, port?: int, database: string, username: string, password: string, sourceTable: string, tableName: string, sslMode?: string, readonly?: bool }): { sessionOut: Struct, connectionUrl: string };

/**
 * Register an Oracle database table for federated queries via ODBC. Requires Oracle Instant Client with ODBC driver installed.
 * @param session — DataFusion session
 * @param host (optional) — Oracle server host
 * @param port (optional) — Oracle listener port
 * @param serviceName (optional) — Oracle service name or SID
 * @param username — Database username
 * @param password — Database password
 * @param schema (optional) — Oracle schema (defaults to username)
 * @param sourceTable — Name of the table in Oracle
 * @param tableName — Name to register in DataFusion
 * @param odbcDriver (optional) — ODBC driver name (e.g., 'Oracle 21 ODBC driver')
 * @param readonly (optional) — Open connection in read-only mode
 * @returns sessionOut — DataFusion session
 * @returns connectionUrl — Generated connection URL (without password)
 * @impure has side effects / drives control flow
 */
declare function dfRegisterOracle({ session: Struct, host?: string, port?: int, serviceName?: string, username: string, password: string, schema?: string, sourceTable: string, tableName: string, odbcDriver?: string, readonly?: bool }): { sessionOut: Struct, connectionUrl: string };

/**
 * Register a PostgreSQL table for federated queries. Uses real database connection for full SQL push-down.
 * @param session — DataFusion session
 * @param host (optional) — PostgreSQL server host
 * @param port (optional) — PostgreSQL server port
 * @param database — Database name
 * @param username — Database username
 * @param password — Database password
 * @param schema (optional) — PostgreSQL schema
 * @param sourceTable — Name of the table in PostgreSQL
 * @param tableName — Name to register in DataFusion
 * @param sslMode (optional) — SSL mode: disable, prefer, require, verify-ca, verify-full
 * @param readonly (optional) — Open connection in read-only mode
 * @returns sessionOut — DataFusion session
 * @returns connectionUrl — Generated connection URL (without password)
 * @impure has side effects / drives control flow
 */
declare function dfRegisterPostgres({ session: Struct, host?: string, port?: int, database: string, username: string, password: string, schema?: string, sourceTable: string, tableName: string, sslMode?: string, readonly?: bool }): { sessionOut: Struct, connectionUrl: string };

/**
 * Register a SQLite database table for federated queries. Uses real database connection.
 * @param session — DataFusion session
 * @param filePath — Path to SQLite database file
 * @param sourceTable — Name of the table in SQLite
 * @param tableName — Name to register in DataFusion
 * @param readonly (optional) — Open database in read-only mode
 * @returns sessionOut — DataFusion session
 * @impure has side effects / drives control flow
 */
declare function dfRegisterSqlite({ session: Struct, filePath: string, sourceTable: string, tableName: string, readonly?: bool }): Struct;


// === Data/DataFusion/Lakes ===

/**
 * Get metadata and history information about a Delta table.
 * @param path — FlowPath to the Delta table directory
 * @param historyLimit (optional) — Max number of history entries to return
 * @returns currentVersion — Latest version number
 * @returns schema — Table schema as typed field metadata
 * @returns history — Version history as array of typed entries
 * @returns partitions — Partition columns
 * @impure has side effects / drives control flow
 */
declare function dfDeltaInfo({ path: Struct, historyLimit?: int }): { currentVersion: int, schema: Struct, history: Struct[], partitions: string[] };

/**
 * Load a specific version or timestamp of a Delta table for point-in-time queries.
 * @param session — DataFusion session
 * @param path — FlowPath to the Delta table directory
 * @param tableName — Name to register in DataFusion
 * @param travelMode (optional) — Mode: 'version' or 'timestamp'
 * @param version (optional) — Version number (when mode is 'version')
 * @param timestamp (optional) — ISO 8601 timestamp (when mode is 'timestamp')
 * @returns sessionOut — DataFusion session
 * @returns loadedVersion — Actual version loaded
 * @impure has side effects / drives control flow
 */
declare function dfDeltaTimeTravel({ session: Struct, path: Struct, tableName: string, travelMode?: string, version?: int, timestamp?: string }): { sessionOut: Struct, loadedVersion: int };

/**
 * Get metadata, snapshots, and history of an Apache Iceberg table from a metadata file.
 * @param warehousePath — FlowPath to the Iceberg table directory
 * @param metadataFile — Relative path to metadata JSON file
 * @returns currentSnapshot — Current snapshot ID
 * @returns schema — Table schema as JSON
 * @returns snapshots — List of all snapshots
 * @returns partitionSpec — Current partition specification
 * @returns properties — Table properties
 * @impure has side effects / drives control flow
 */
declare function dfIcebergInfo({ warehousePath: Struct, metadataFile: string }): { currentSnapshot: string, schema: Struct, snapshots: Struct, partitionSpec: Struct, properties: Struct };

/**
 * Load a specific snapshot of an Iceberg table for point-in-time queries.
 * @param session — DataFusion session
 * @param warehousePath — FlowPath to the Iceberg table directory
 * @param metadataFile — Relative path to metadata JSON file
 * @param tableName — Name to register in DataFusion
 * @param travelMode (optional) — Mode: 'snapshot' or 'timestamp'
 * @param snapshotId (optional) — Snapshot ID (when mode is 'snapshot')
 * @param timestampMs (optional) — Unix timestamp in milliseconds (when mode is 'timestamp')
 * @returns sessionOut — DataFusion session
 * @returns loadedSnapshot — Actual snapshot ID that was loaded
 * @impure has side effects / drives control flow
 */
declare function dfIcebergTimeTravel({ session: Struct, warehousePath: Struct, metadataFile: string, tableName: string, travelMode?: string, snapshotId?: string, timestampMs?: int }): { sessionOut: Struct, loadedSnapshot: string };

/**
 * Register a Delta Lake table in DataFusion using a FlowPath. Requires the 'delta' feature.
 * @param session — DataFusion session
 * @param path — FlowPath to the Delta table directory
 * @param tableName — Name to register in DataFusion
 * @param version (optional) — Specific version to load (-1 for latest)
 * @returns sessionOut — DataFusion session
 * @returns tableVersion — Actual version loaded
 * @impure has side effects / drives control flow
 */
declare function dfRegisterDelta({ session: Struct, path: Struct, tableName: string, version?: int }): { sessionOut: Struct, tableVersion: int };

/**
 * Register Hive-partitioned Parquet files as a table in DataFusion using a FlowPath.
 * @param session — DataFusion session
 * @param path — FlowPath to root directory of partitioned Parquet files
 * @param tableName — Name to register in DataFusion
 * @returns sessionOut — DataFusion session
 * @impure has side effects / drives control flow
 */
declare function dfRegisterHiveParquet({ session: Struct, path: Struct, tableName: string }): Struct;

/**
 * Register an Apache Iceberg table in DataFusion from a metadata file. Supports schema evolution and partition pruning.
 * @param session — DataFusion session
 * @param warehousePath — FlowPath to the Iceberg table metadata directory
 * @param metadataFile — Relative path to metadata JSON file (e.g., 'metadata/v1.metadata.json')
 * @param tableName — Name to register in DataFusion
 * @returns sessionOut — DataFusion session
 * @returns currentSnapshot — Current snapshot ID
 * @returns schemaInfo — Table schema field count
 * @impure has side effects / drives control flow
 */
declare function dfRegisterIceberg({ session: Struct, warehousePath: Struct, metadataFile: string, tableName: string }): { sessionOut: Struct, currentSnapshot: string, schemaInfo: int };

/**
 * Register partitioned JSON/NDJSON files as a table in DataFusion using a FlowPath.
 * @param session — DataFusion session
 * @param path — FlowPath to JSON files
 * @param tableName — Name to register
 * @param fileExtension (optional) — File extension to match
 * @returns sessionOut — DataFusion session
 * @impure has side effects / drives control flow
 */
declare function dfRegisterPartitionedJson({ session: Struct, path: Struct, tableName: string, fileExtension?: string }): Struct;

/**
 * Write query results to a new or existing Delta Lake table using FlowPath. Supports append, overwrite modes.
 * @param session — DataFusion session
 * @param query — SQL query to execute
 * @param path — FlowPath for the Delta table directory
 * @param mode (optional) — Write mode: append, overwrite, error, ignore
 * @param partitionBy (optional) — Columns to partition by (comma-separated)
 * @returns sessionOut — DataFusion session
 * @returns rowsWritten — Number of rows written
 * @returns newVersion — Version number after write
 * @impure has side effects / drives control flow
 */
declare function dfWriteDelta({ session: Struct, query: string, path: Struct, mode?: string, partitionBy?: string }): { sessionOut: Struct, rowsWritten: int, newVersion: int };


// === Data/DataFusion/Time ===

/**
 * Convert a DateTime (ISO 8601 string) to SQL timestamp literal for use in DataFusion queries.
 * @param datetime — DateTime value (ISO 8601 string format)
 * @returns timestampLiteral — SQL timestamp literal (e.g., TIMESTAMP '2024-01-15 10:30:00')
 * @returns epochMicros — Timestamp as microseconds since Unix epoch
 */
declare function dfDatetimeToTimestamp({ datetime: Date }): { timestampLiteral: string, epochMicros: int };

/**
 * Generate a SQL WHERE clause for filtering by time range. Supports relative time expressions.
 * @param timestampColumn — Name of the timestamp column to filter
 * @param startTime (optional) — Start of range (ISO 8601 or relative: '-1d', '-24h', '-30m')
 * @param endTime (optional) — End of range (ISO 8601, 'now', or relative)
 * @returns whereClause — SQL WHERE clause fragment
 * @returns startTimestamp — Resolved start timestamp literal
 * @returns endTimestamp — Resolved end timestamp literal
 */
declare function dfTimeRangeFilter({ timestampColumn: string, startTime?: string, endTime?: string }): { whereClause: string, startTimestamp: string, endTimestamp: string };


// === Data/DataFusion/Tools ===

/**
 * Get the schema (column names and types) of a table in a DataFusion session.
 * @param session — DataFusion session containing the table
 * @param tableName (optional) — Name of the table to describe
 * @returns schema — Table schema description (column names and types)
 * @impure has side effects / drives control flow
 */
declare function dfDescribeTable({ session: Struct, tableName?: string }): string;

/**
 * Execute a SQL query and return results as formatted text. Ideal for agent-driven data exploration.
 * @param session — DataFusion session to query
 * @param query (optional) — SQL query to execute
 * @returns result — Query results formatted as markdown table
 * @returns table — Query results as CSVTable for further processing
 * @returns rowCount — Number of rows returned
 * @impure has side effects / drives control flow
 */
declare function dfExecuteSql({ session: Struct, query?: string }): { result: string, table: Struct, rowCount: int };

/**
 * List all tables registered in a DataFusion session. Returns array of table names.
 * @param session — DataFusion session to query
 * @returns tables — Array of table info objects
 * @returns tableNames — Simple array of table names (for queries)
 * @returns summary — Human-readable summary of available tables
 * @impure has side effects / drives control flow
 */
declare function dfListTables({ session: Struct }): { tables: Struct[], tableNames: string[], summary: string };


// === Data/Database ===

/**
 * Open a local database
 * @param name — Name of the Table
 * @param userScoped (optional) — Store database in user directory instead of project directory
 * @param batchSize (optional) — Number of items to buffer before flushing writes to storage. 0 = no buffering.
 * @returns database — Database Connection Reference
 * @impure has side effects / drives control flow
 */
declare function openLocalDb({ name: string, userScoped?: bool, batchSize?: int }): Struct;


// === Data/Database/Delete ===

/**
 * Delete rows from a database table and return the removed rows
 * @param database — Database Connection Reference
 * @param filter (optional) — Optional SQL filter. Leave empty to delete all rows.
 * @returns deletedValues — Rows that were deleted
 * @impure has side effects / drives control flow
 */
declare function filterDeleteLocalDb({ database: Struct, filter?: string }): Struct[];

/**
 * Purge Database
 * @param database — Database Connection Reference
 * @impure has side effects / drives control flow
 */
declare function purgeLocalDb({ database: Struct }): void;


// === Data/Database/Graph ===

/**
 * Creates a new graph overlay definition over existing database tables
 * @param overlay — The graph overlay definition (JSON)
 * @param userScoped (optional) — Store in user-scoped database
 * @returns errorMessage — Error details
 * @returns overlayId — ID of the created overlay
 * @impure has side effects / drives control flow
 */
declare function createGraphOverlay({ overlay: Struct, userScoped?: bool }): { errorMessage: string, overlayId: string };

/**
 * Deletes a graph overlay definition (does not drop underlying tables)
 * @param overlayId — ID of the overlay to delete
 * @param userScoped (optional) — Delete from user-scoped database
 * @returns errorMessage — Error details
 * @impure has side effects / drives control flow
 */
declare function dropGraphOverlay({ overlayId: string, userScoped?: bool }): string;

/**
 * Opens an existing graph overlay and returns a connection for querying
 * @param overlayId — ID of the graph overlay to open
 * @param userScoped (optional) — Use user-scoped database instead of project-scoped
 * @returns errorMessage — Error details
 * @returns graph — Graph connection reference for query nodes
 * @impure has side effects / drives control flow
 */
declare function openGraphOverlay({ overlayId: string, userScoped?: bool }): { errorMessage: string, graph: Struct };


// === Data/Database/Graph/Meta ===

/**
 * Retrieves the schema (labels and properties) of a graph overlay
 * @param graph — Graph connection reference
 * @returns schema — Graph schema with labels and properties
 * @impure has side effects / drives control flow
 */
declare function graphSchema({ graph: Struct }): Struct;

/**
 * Lists all graph overlay definitions in the database
 * @param userScoped (optional) — List overlays from user-scoped database
 * @returns overlayIds — List of overlay IDs
 * @returns overlayNames — List of overlay names
 * @impure has side effects / drives control flow
 */
declare function listGraphOverlays({ userScoped?: bool }): { overlayIds: string[], overlayNames: string[] };


// === Data/Database/Graph/Query ===

/**
 * Executes a Cypher query against the graph overlay
 * @param graph — Graph connection reference
 * @param query — Cypher query string
 * @param params — Query parameters (JSON object)
 * @param limit (optional) — Maximum number of results
 * @returns errorMessage — Error details
 * @returns results — Query results as JSON array
 * @impure has side effects / drives control flow
 */
declare function graphCypherQuery({ graph: Struct, query: string, params: Struct, limit?: int }): { errorMessage: string, results: Struct[] };

/**
 * Finds neighbor nodes by traversing edges from a seed node
 * @param graph — Graph connection reference
 * @param label — Label of the seed node
 * @param nodeId — ID of the seed node
 * @param depth (optional) — Maximum traversal depth (1-5)
 * @param direction (optional) — Traversal direction: outgoing, incoming, or both
 * @param limit (optional) — Maximum number of results
 * @returns errorMessage — Error details
 * @returns resultNodes — Discovered nodes
 * @returns resultEdges — Discovered edges
 * @impure has side effects / drives control flow
 */
declare function graphNeighbors({ graph: Struct, label: string, nodeId: string, depth?: int, direction?: string, limit?: int }): { errorMessage: string, resultNodes: Struct[], resultEdges: Struct[] };

/**
 * Executes a SQL query against graph overlay tables via DataFusion
 * @param graph — Graph connection reference
 * @param query — SQL query string
 * @param limit (optional) — Maximum number of results
 * @returns errorMessage — Error details
 * @returns results — Query results as JSON array
 * @impure has side effects / drives control flow
 */
declare function graphSqlQuery({ graph: Struct, query: string, limit?: int }): { errorMessage: string, results: Struct[] };

/**
 * Extracts a subgraph around seed nodes for visualization
 * @param graph — Graph connection reference
 * @param seedLabels — Labels of seed nodes (parallel array with Seed IDs)
 * @param seedIds — IDs of seed nodes (parallel array with Seed Labels)
 * @param depth (optional) — Maximum traversal depth (1-5)
 * @param limit (optional) — Maximum number of results
 * @returns errorMessage — Error details
 * @returns payload — Subgraph data with nodes and edges
 * @returns truncated — Whether the result was truncated
 * @impure has side effects / drives control flow
 */
declare function graphSubgraph({ graph: Struct, seedLabels: string[], seedIds: string[], depth?: int, limit?: int }): { errorMessage: string, payload: Struct, truncated: bool };


// === Data/Database/Graph/Write ===

/**
 * Inserts or updates an edge in the graph overlay's underlying edge table
 * @param graph — Graph connection reference
 * @param label — Label of the edge type to upsert into
 * @param value — Edge data as JSON object (must include src/dst columns)
 * @returns errorMessage — Error details
 * @impure has side effects / drives control flow
 */
declare function upsertGraphEdge({ graph: Struct, label: string, value: Struct }): string;

/**
 * Inserts or updates a node in the graph overlay's underlying table
 * @param graph — Graph connection reference
 * @param label — Label of the node type to upsert into
 * @param value — Node data as JSON object
 * @returns errorMessage — Error details
 * @impure has side effects / drives control flow
 */
declare function upsertGraphNode({ graph: Struct, label: string, value: Struct }): string;


// === Data/Database/Insert ===

/**
 * Inserts multiple items at once. Faster than Upsert but might produce duplicates.
 * @param database — Database Connection Reference
 * @param value — Value to Insert
 * @returns errorMessage — Error details
 * @impure has side effects / drives control flow
 */
declare function batchInsertLocalDb({ database: Struct, value: Struct[] }): string;

/**
 * Inserts if the Item does not exist, Updates if it does
 * @param database — Database Connection Reference
 * @param idRow — The ID Column
 * @param value — Value to Insert
 * @returns errorMessage — Error details
 * @impure has side effects / drives control flow
 */
declare function batchUpsertLocalDb({ database: Struct, idRow: string, value: Struct[] }): string;

/**
 * Inserts multiple items at once. Faster than Upsert but might produce duplicates.
 * @param database — Database Connection Reference
 * @param csv — CSV Path
 * @param chunkSize (optional) — Chunk Size for Buffered Read
 * @param delimiter (optional) — Delimiter for CSV
 * @returns errorMessage — Error details
 * @impure has side effects / drives control flow
 */
declare function csvInsertLocalDb({ database: Struct, csv: Struct, chunkSize?: int, delimiter?: string }): string;

/**
 * Faster than Upsert, but might write duplicate items.
 * @param database — Database Connection Reference
 * @param value — Value to Insert
 * @returns errorMessage — Error details
 * @impure has side effects / drives control flow
 */
declare function insertLocalDb({ database: Struct, value: Struct }): string;

/**
 * Reads a LabVIEW TDMS file and batch-inserts its channel data as rows into a vector database.
 * @param database — Database Connection Reference
 * @param tdmsPath — Path to the TDMS file
 * @param chunkSize (optional) — Chunk Size for buffered Arrow inserts
 * @impure has side effects / drives control flow
 */
declare function tdmsInsertLocalDb({ database: Struct, tdmsPath: Struct, chunkSize?: int }): void;

/**
 * Inserts if the Item does not exist, Updates if it does
 * @param database — Database Connection Reference
 * @param idRow — The ID Column
 * @param value — Value to Insert
 * @returns errorMessage — Error details
 * @impure has side effects / drives control flow
 */
declare function upsertLocalDb({ database: Struct, idRow: string, value: Struct }): string;


// === Data/Database/Meta ===

/**
 * Count Items
 * @param database — Database Connection Reference
 * @param filter (optional) — Optional SQL Filter
 * @returns count — Found Items Count
 * @impure has side effects / drives control flow
 */
declare function countLocalDb({ database: Struct, filter?: string }): int;

/**
 * Lists all indices on a database table
 * @param database — Database Connection Reference
 * @returns indices — List of indices on the table
 * @impure has side effects / drives control flow
 */
declare function listIndicesDb({ database: Struct }): Struct[];

/**
 * List Content
 * @param database — Database Connection Reference
 * @param limit (optional) — Limit
 * @param offset (optional) — Offset
 * @returns values — Found Items
 * @impure has side effects / drives control flow
 */
declare function listLocalDb({ database: Struct, limit?: int, offset?: int }): Struct[];

/**
 * Lists all available table names in the database location
 * @param userScoped (optional) — List tables from user directory instead of project directory
 * @returns tables — List of table names
 * @impure has side effects / drives control flow
 */
declare function listTablesDb({ userScoped?: bool }): string[];

/**
 * Get Local Database Schema
 * @param database — Database Connection Reference
 * @returns schema — Local Database Schema
 * @impure has side effects / drives control flow
 */
declare function schemaLocalDb({ database: Struct }): Struct;


// === Data/Database/Optimization ===

/**
 * Remove an index from a database table
 * @param database — Database Connection Reference
 * @param indexName (optional) — Name of the index to drop
 * @impure has side effects / drives control flow
 */
declare function dropIndexDb({ database: Struct, indexName?: string }): void;

/**
 * Flush any buffered writes to storage immediately
 * @param database — Database Connection Reference
 * @returns errorMessage — Error details
 * @impure has side effects / drives control flow
 */
declare function flushLocalDb({ database: Struct }): string;

/**
 * Build Index
 * @param database — Database Connection Reference
 * @param column (optional) — Column to Index
 * @param type (optional) — Index Type to build
 * @impure has side effects / drives control flow
 */
declare function indexLocalDb({ database: Struct, column?: string, type?: string }): void;

/**
 * Optimize and Update the Database
 * @param database — Database Connection Reference
 * @param keepVersions (optional) — Otherwise deletes old versions
 * @impure has side effects / drives control flow
 */
declare function optimizeLocalDb({ database: Struct, keepVersions?: bool }): void;


// === Data/Database/Schema ===

/**
 * Adds a column using a typed SQL expression (e.g. 0, '', CAST(NULL AS STRING)). LanceDB rejects bare NULL — wrap it in CAST(... AS <type>). Supported types: int, bigint, float, double, string, binary, boolean, date, timestamp.
 * @param database — Database Connection Reference
 * @param columnName (optional) — Name of the column to add
 * @param sqlExpression (optional) — Typed SQL expression used to populate existing rows. Examples: 0, '', CAST(NULL AS STRING). Bare NULL is rejected; LanceDB supports int, bigint, float, double, string, binary, boolean, date, timestamp.
 * @returns schema — Updated database schema
 * @impure has side effects / drives control flow
 */
declare function addColumnLocalDb({ database: Struct, columnName?: string, sqlExpression?: string }): Struct;

/**
 * Drops a column from the database table.
 * @param database — Database Connection Reference
 * @param columnName (optional) — Name of the column to drop
 * @returns schema — Updated database schema
 * @impure has side effects / drives control flow
 */
declare function dropColumnLocalDb({ database: Struct, columnName?: string }): Struct;

/**
 * Marks a column as optional (nullable).
 * @param database — Database Connection Reference
 * @param columnName (optional) — Name of the column
 * @param optional (optional) — True = nullable, false = required
 * @returns schema — Updated database schema
 * @impure has side effects / drives control flow
 */
declare function makeColumnOptionalLocalDb({ database: Struct, columnName?: string, optional?: bool }): Struct;


// === Data/Database/Search ===

/**
 * Filter Database
 * @param database — Database Connection Reference
 * @param filter (optional) — Optional SQL Filter
 * @param limit (optional) — Limit
 * @param offset (optional) — Offset
 * @returns values — Found Items
 * @impure has side effects / drives control flow
 */
declare function filterLocalDb({ database: Struct, filter?: string, limit?: int, offset?: int }): Struct[];

/**
 * Searches the Database using Full-Text Search
 * @param database — Database Connection Reference
 * @param search (optional) — Full Text Search Term
 * @param fields (optional) — Column names to search with FTS (searches all indexed columns if empty)
 * @param filter (optional) — Optional SQL Filter
 * @param limit (optional) — Limit
 * @param offset (optional) — Offset
 * @returns values — Found Items
 * @impure has side effects / drives control flow
 */
declare function ftsSearchLocalDb({ database: Struct, search?: string, fields?: string[], filter?: string, limit?: int, offset?: int }): Struct[];

/**
 * Searches the Database using both Vector and Full-Text Search
 * @param database — Database Connection Reference
 * @param search (optional) — Full Text Search Term
 * @param vector — Vector to Search
 * @param fields (optional) — Column names for both vector (first) and FTS search
 * @param filter (optional) — Optional SQL Filter
 * @param rerank (optional) — Should the items be reranked using RRF?
 * @param limit (optional) — Limit
 * @param offset (optional) — Offset
 * @returns values — Found Items
 * @impure has side effects / drives control flow
 */
declare function hybridSearchLocalDb({ database: Struct, search?: string, vector: float[], fields?: string[], filter?: string, rerank?: bool, limit?: int, offset?: int }): Struct[];

/**
 * Searches the Database based on a Vector
 * @param database — Database Connection Reference
 * @param vector — Vector to Search
 * @param filter (optional) — Optional SQL Filter
 * @param limit (optional) — Limit
 * @param offset (optional) — Offset
 * @returns values — Found Items
 * @impure has side effects / drives control flow
 */
declare function vectorSearchLocalDb({ database: Struct, vector: float[], filter?: string, limit?: int, offset?: int }): Struct[];


// === Data/Databricks ===

/**
 * Cancel a running job
 * @param provider — Databricks provider
 * @param runId — The ID of the job run to cancel
 * @returns errorMessage — Error details if the request fails
 * @impure has side effects / drives control flow
 */
declare function dataDatabricksCancelJobRun({ provider: Struct, runId: int }): string;

/**
 * Execute a SQL statement on a Databricks SQL warehouse. Supports SELECT, INSERT, UPDATE, DELETE, and DDL statements.
 * @param provider — Databricks provider
 * @param warehouseId — The SQL warehouse ID to execute the statement on
 * @param statement — The SQL statement to execute
 * @param catalog (optional) — Optional: The catalog to use (Unity Catalog)
 * @param schema (optional) — Optional: The schema to use
 * @param rowLimit (optional) — Maximum number of rows to return (default: 10000)
 * @param waitTimeout (optional) — Timeout in seconds for synchronous execution (default: 50s, max: 50s)
 * @returns result — SQL execution result
 * @returns rows — Result rows as JSON array
 * @returns rowCount — Number of rows returned
 * @returns errorMessage — Error details if the request fails
 * @impure has side effects / drives control flow
 */
declare function dataDatabricksExecuteSql({ provider: Struct, warehouseId: string, statement: string, catalog?: string, schema?: string, rowLimit?: int, waitTimeout?: string }): { result: Struct, rows: Struct[], rowCount: int, errorMessage: string };

/**
 * Get details of a specific cluster by ID
 * @param provider — Databricks provider
 * @param clusterId — The ID of the cluster to retrieve
 * @returns cluster — Cluster details
 * @returns errorMessage — Error details if the request fails
 * @impure has side effects / drives control flow
 */
declare function dataDatabricksGetCluster({ provider: Struct, clusterId: string }): { cluster: Struct, errorMessage: string };

/**
 * Get the status of a job run
 * @param provider — Databricks provider
 * @param runId — The ID of the job run
 * @returns run — Job run details
 * @returns isRunning — Whether the job is still running
 * @returns isSuccessful — Whether the job completed successfully
 * @returns errorMessage — Error details if the request fails
 * @impure has side effects / drives control flow
 */
declare function dataDatabricksGetJobRun({ provider: Struct, runId: int }): { run: Struct, isRunning: bool, isSuccessful: bool, errorMessage: string };

/**
 * List all clusters in the Databricks workspace
 * @param provider — Databricks provider
 * @returns clusters — Array of clusters
 * @returns count — Number of clusters returned
 * @returns errorMessage — Error details if the request fails
 * @impure has side effects / drives control flow
 */
declare function dataDatabricksListClusters({ provider: Struct }): { clusters: Struct[], count: int, errorMessage: string };

/**
 * List all jobs in the Databricks workspace
 * @param provider — Databricks provider
 * @param limit (optional) — Maximum number of jobs to return (default: 25, max: 100)
 * @param offset (optional) — Offset for pagination
 * @param name (optional) — Optional: Filter jobs by name (substring match)
 * @returns jobs — Array of jobs
 * @returns count — Number of jobs returned
 * @returns hasMore — Whether there are more jobs available
 * @returns errorMessage — Error details if the request fails
 * @impure has side effects / drives control flow
 */
declare function dataDatabricksListJobs({ provider: Struct, limit?: int, offset?: int, name?: string }): { jobs: Struct[], count: int, hasMore: bool, errorMessage: string };

/**
 * List all SQL warehouses in the Databricks workspace
 * @param provider — Databricks provider
 * @returns warehouses — Array of SQL warehouses
 * @returns count — Number of warehouses returned
 * @returns errorMessage — Error details if the request fails
 * @impure has side effects / drives control flow
 */
declare function dataDatabricksListSqlWarehouses({ provider: Struct }): { warehouses: Struct[], count: int, errorMessage: string };

/**
 * Connect to Databricks using OAuth. The workspace URL determines the OAuth endpoints.
 * @param workspaceUrl — Your Databricks workspace URL (e.g., https://adb-1234567890123456.7.azuredatabricks.net)
 * @returns provider — Databricks provider with authentication
 */
declare function dataDatabricksProviderOauth({ workspaceUrl: string }): Struct;

/**
 * Connect to Databricks using a Personal Access Token. Generate one in your Databricks workspace under User Settings > Developer > Access tokens.
 * @param token — Your Databricks Personal Access Token
 * @param workspaceUrl — Your Databricks workspace URL (e.g., https://adb-1234567890123456.7.azuredatabricks.net or https://dbc-a1b2c3d4-e5f6.cloud.databricks.com)
 * @returns provider — Databricks provider with authentication
 */
declare function dataDatabricksProviderPat({ token: string, workspaceUrl: string }): Struct;

/**
 * Connect to Databricks using OAuth M2M (Machine-to-Machine) authentication with a service principal. Ideal for automated workflows and CI/CD pipelines.
 * @param clientId — The service principal's client ID (application ID)
 * @param clientSecret — The service principal's OAuth secret
 * @param workspaceUrl — Your Databricks workspace URL for workspace-level operations
 * @param accountId (optional) — Optional: Databricks account ID for account-level operations. Leave empty for workspace-level only.
 * @returns provider — Databricks provider with authentication
 * @returns errorMessage — Error message if authentication fails
 * @impure has side effects / drives control flow
 */
declare function dataDatabricksProviderServicePrincipal({ clientId: string, clientSecret: string, workspaceUrl: string, accountId?: string }): { provider: Struct, errorMessage: string };

/**
 * Connect to Databricks using an externally managed access token. Use this for tokens obtained from OAuth flows or service principals.
 * @param token — Databricks access token (OAuth or PAT)
 * @param workspaceUrl — Your Databricks workspace URL
 * @returns provider — Databricks provider with authentication
 */
declare function dataDatabricksProviderToken({ token: string, workspaceUrl: string }): Struct;

/**
 * Trigger a job run immediately
 * @param provider — Databricks provider
 * @param jobId — The ID of the job to run
 * @param jobParameters (optional) — Optional: JSON object with job parameters
 * @returns runId — The ID of the job run
 * @returns errorMessage — Error details if the request fails
 * @impure has side effects / drives control flow
 */
declare function dataDatabricksRunJob({ provider: Struct, jobId: int, jobParameters?: Struct }): { runId: int, errorMessage: string };

/**
 * Start a terminated cluster
 * @param provider — Databricks provider
 * @param clusterId — The ID of the cluster to start
 * @returns errorMessage — Error details if the request fails
 * @impure has side effects / drives control flow
 */
declare function dataDatabricksStartCluster({ provider: Struct, clusterId: string }): string;

/**
 * Start a stopped SQL warehouse
 * @param provider — Databricks provider
 * @param warehouseId — The ID of the SQL warehouse to start
 * @returns errorMessage — Error details if the request fails
 * @impure has side effects / drives control flow
 */
declare function dataDatabricksStartSqlWarehouse({ provider: Struct, warehouseId: string }): string;

/**
 * Terminate a running cluster
 * @param provider — Databricks provider
 * @param clusterId — The ID of the cluster to terminate
 * @returns errorMessage — Error details if the request fails
 * @impure has side effects / drives control flow
 */
declare function dataDatabricksStopCluster({ provider: Struct, clusterId: string }): string;

/**
 * Stop a running SQL warehouse
 * @param provider — Databricks provider
 * @param warehouseId — The ID of the SQL warehouse to stop
 * @returns errorMessage — Error details if the request fails
 * @impure has side effects / drives control flow
 */
declare function dataDatabricksStopSqlWarehouse({ provider: Struct, warehouseId: string }): string;


// === Data/Databricks/DBFS ===

/**
 * Get the status (metadata) of a file or directory in DBFS
 * @param provider — Databricks provider
 * @param path — DBFS path to check
 * @returns fileInfo — File or directory information
 * @returns exists — Whether the path exists
 * @returns errorMessage — Error details if the request fails
 * @impure has side effects / drives control flow
 */
declare function dataDatabricksGetDbfsStatus({ provider: Struct, path: string }): { fileInfo: Struct, exists: bool, errorMessage: string };

/**
 * List files and directories in the Databricks File System (DBFS)
 * @param provider — Databricks provider
 * @param path (optional) — DBFS path to list (e.g., /FileStore, /mnt)
 * @returns files — Array of files and directories
 * @returns count — Number of items
 * @returns errorMessage — Error details if the request fails
 * @impure has side effects / drives control flow
 */
declare function dataDatabricksListDbfs({ provider: Struct, path?: string }): { files: Struct[], count: int, errorMessage: string };

/**
 * Read the contents of a file from DBFS. Returns base64 encoded content for binary files.
 * @param provider — Databricks provider
 * @param path — DBFS path of the file to read
 * @param offset (optional) — Byte offset to start reading from
 * @param length (optional) — Number of bytes to read (max 1MB)
 * @returns content — File content (base64 encoded for binary files)
 * @returns bytesRead — Number of bytes read
 * @returns errorMessage — Error details if the request fails
 * @impure has side effects / drives control flow
 */
declare function dataDatabricksReadDbfs({ provider: Struct, path: string, offset?: int, length?: int }): { content: string, bytesRead: int, errorMessage: string };


// === Data/Databricks/Unity Catalog ===

/**
 * List all catalogs in Unity Catalog
 * @param provider — Databricks provider
 * @returns catalogs — Array of catalogs
 * @returns count — Number of catalogs
 * @returns errorMessage — Error details if the request fails
 * @impure has side effects / drives control flow
 */
declare function dataDatabricksListCatalogs({ provider: Struct }): { catalogs: Struct[], count: int, errorMessage: string };

/**
 * List all schemas in a catalog
 * @param provider — Databricks provider
 * @param catalogName — The name of the catalog
 * @returns schemas — Array of schemas
 * @returns count — Number of schemas
 * @returns errorMessage — Error details if the request fails
 * @impure has side effects / drives control flow
 */
declare function dataDatabricksListSchemas({ provider: Struct, catalogName: string }): { schemas: Struct[], count: int, errorMessage: string };

/**
 * List all tables in a schema
 * @param provider — Databricks provider
 * @param catalogName — The name of the catalog
 * @param schemaName — The name of the schema
 * @returns tables — Array of tables
 * @returns count — Number of tables
 * @returns errorMessage — Error details if the request fails
 * @impure has side effects / drives control flow
 */
declare function dataDatabricksListTables({ provider: Struct, catalogName: string, schemaName: string }): { tables: Struct[], count: int, errorMessage: string };


// === Data/Excel ===

/**
 * Extracts tables from an Excel worksheet
 * @param file — Excel file
 * @param sheet (optional) — Worksheet name (optional - if empty, extracts from all sheets)
 * @param extractConfig (optional) — Extract Config
 * @returns tables — Extracted Vec<Table>
 * @impure has side effects / drives control flow
 */
declare function dataExcelExtractTables({ file: Struct, sheet?: string, extractConfig?: Struct }): Struct[];

/**
 * Uses AI to intelligently extract tables from complex Excel worksheets with unusual layouts
 * @param model — AI model for analysis
 * @param file — Excel file
 * @param sheet (optional) — Worksheet name (optional - if empty, extracts from all sheets)
 * @param userHint (optional) — Optional guidance for the AI (e.g., 'The table starts at row 5', 'Skip rows with TOTAL')
 * @param config (optional) — AI extraction configuration
 * @returns tables — Extracted tables
 * @returns reasoning — AI's explanation of extraction strategy
 * @impure has side effects / drives control flow
 */
declare function dataExcelExtractTablesAi({ model: Struct, file: Struct, sheet?: string, userHint?: string, config?: Struct }): { tables: Struct[], reasoning: string };

/**
 * Read a single cell value from an XLSX sheet
 * @param file — Source XLSX file
 * @param sheet (optional) — Worksheet name
 * @param row (optional) — Row number (1-based)
 * @param col (optional) — Column letters or number (1-based)
 * @returns fileOut — Pass-through XLSX path
 * @returns value — Cell value (raw string)
 * @returns found — Cell exists and has a value
 * @impure has side effects / drives control flow
 */
declare function excelReadCell({ file: Struct, sheet?: string, row?: string, col?: string }): { fileOut: Struct, value: string, found: bool };

/**
 * Delete one or more columns from an XLSX sheet
 * @param file — Target XLSX file
 * @param sheet (optional) — Worksheet name
 * @param col (optional) — Column letter(s) or 1-based number
 * @param count (optional) — How many columns to remove
 * @returns fileOut — Updated XLSX path
 * @returns ok — Operation success
 * @impure has side effects / drives control flow
 */
declare function excelRemoveColumn({ file: Struct, sheet?: string, col?: string, count?: string }): { fileOut: Struct, ok: bool };

/**
 * Delete one or more rows from an XLSX sheet
 * @param file — Target XLSX file
 * @param sheet (optional) — Worksheet name
 * @param row (optional) — Start row (1-based)
 * @param count (optional) — How many rows to remove
 * @returns fileOut — Updated XLSX path
 * @returns ok — Operation success
 * @impure has side effects / drives control flow
 */
declare function excelRemoveRow({ file: Struct, sheet?: string, row?: string, count?: string }): { fileOut: Struct, ok: bool };

/**
 * Write/update a single cell value in an XLSX sheet
 * @param file — Target XLSX file
 * @param sheet (optional) — Worksheet name
 * @param row (optional) — Row number (1-based)
 * @param col (optional) — Column (letter(s) like A, AA, or 1-based number)
 * @param value (optional) — Value to write (string)
 * @returns fileOut — Updated XLSX path
 * @impure has side effects / drives control flow
 */
declare function excelWriteCell({ file: Struct, sheet?: string, row?: string, col?: string, value?: string }): Struct;

/**
 * Write/update a single cell value in an XLSX sheet (HTML)
 * @param file — Target XLSX file
 * @param sheet (optional) — Worksheet name
 * @param row (optional) — Row number (1-based)
 * @param col (optional) — Column (letter(s) like A, AA, or 1-based number)
 * @param value (optional) — Value to write (string)
 * @returns fileOut — Updated XLSX path
 * @impure has side effects / drives control flow
 */
declare function excelWriteCellHtml({ file: Struct, sheet?: string, row?: string, col?: string, value?: string }): Struct;

/**
 * Duplicate a worksheet within the same file
 * @param file — The .xlsx file to modify
 * @param sourceSheet (optional) — Name or 0-based index of the source sheet
 * @param newName (optional) — Name for the copied sheet (optional)
 * @param ifExists (optional) — Behavior if the destination name exists
 * @returns copied — Whether a new sheet was created
 * @returns sourceName — Resolved source sheet name
 * @returns finalName — Actual name of the new sheet
 * @impure has side effects / drives control flow
 */
declare function filesSpreadsheetCopyWorksheet({ file: Struct, sourceSheet?: string, newName?: string, ifExists?: string }): { copied: bool, sourceName: string, finalName: string };

/**
 * List worksheet names using calamine
 * @param file — Spreadsheet file to inspect
 * @returns sheetNames — All worksheet names
 * @returns count — Number of sheets
 */
declare function filesSpreadsheetGetSheetNames({ file: Struct }): { sheetNames: string[], count: int };

/**
 * Insert one or more columns into a worksheet
 * @param file — The .xlsx file to modify
 * @param sheetName (optional) — Target worksheet name
 * @param column (optional) — Target column (letter like 'B' or index like '2')
 * @param position (optional) — Insert before or after the target column
 * @param numColumns (optional) — How many columns to insert
 * @param adjustReferences (optional) — Adjust formulas across workbook
 * @returns inserted — Whether columns were inserted
 * @returns finalColumnIndex — 1-based column index used
 * @returns finalColumnLetter — Excel letter for final index
 * @returns totalColumnsInserted — How many columns were inserted
 * @impure has side effects / drives control flow
 */
declare function filesSpreadsheetInsertColumn({ file: Struct, sheetName?: string, column?: string, position?: string, numColumns?: int, adjustReferences?: bool }): { inserted: bool, finalColumnIndex: int, finalColumnLetter: string, totalColumnsInserted: int };

/**
 * Insert one or more rows into a worksheet
 * @param file — The .xlsx file to modify
 * @param sheetName (optional) — Target worksheet name
 * @param row (optional) — 1-based target row index
 * @param position (optional) — Insert before or after the target row
 * @param numRows (optional) — How many rows to insert
 * @param adjustReferences (optional) — Adjust formulas across workbook
 * @returns inserted — Whether rows were inserted
 * @returns finalRowIndex — 1-based row index used
 * @returns totalRowsInserted — How many rows were inserted
 * @impure has side effects / drives control flow
 */
declare function filesSpreadsheetInsertRow({ file: Struct, sheetName?: string, row?: int, position?: string, numRows?: int, adjustReferences?: bool }): { inserted: bool, finalRowIndex: int, totalRowsInserted: int };

/**
 * Creates a new worksheet (tab) inside an existing .xlsx file
 * @param file — The .xlsx file to modify
 * @param sheetName (optional) — Desired worksheet name
 * @param ifExists (optional) — What to do if the sheet already exists
 * @returns created — Whether a new sheet was created
 * @returns finalName — Actual sheet name used
 * @impure has side effects / drives control flow
 */
declare function filesSpreadsheetNewWorksheet({ file: Struct, sheetName?: string, ifExists?: string }): { created: bool, finalName: string };


// === Data/Excel/Rows ===

/**
 * Return a single row as a struct (1-based index)
 * @param table — CSVTable to read
 * @param rowIndex (optional) — 1-based row index (>=1)
 * @returns row — Row as struct
 * @returns actualRowIndex — Echo of requested index
 */
declare function tablesGetRowByIndex({ table: Struct, rowIndex?: int }): { row: Struct, actualRowIndex: int };


// === Data/Files ===

/**
 * Converts a PathBuf to a Path
 * @param pathbuf — Input PathBuf
 * @returns path — Output Path
 * @impure has side effects / drives control flow
 */
declare function pathbufToPath({ pathbuf: Path }): Struct;


// === Data/Files/Content ===

/**
 * Reads the content of a file Fto bytes
 * @param path — FlowPath
 * @returns content — The content of the file as bytes
 * @impure has side effects / drives control flow
 */
declare function readToBytes({ path: Struct }): bytes[];

/**
 * Reads the content of a file to a string
 * @param path — FlowPath
 * @returns content — The content of the file as a string
 * @impure has side effects / drives control flow
 */
declare function readToString({ path: Struct }): string;

/**
 * Writes bytes to a file
 * @param path — FlowPath
 * @param content — The content to write as bytes
 * @impure has side effects / drives control flow
 */
declare function writeBytes({ path: Struct, content: bytes[] }): void;

/**
 * Writes a string to a file
 * @param path — FlowPath
 * @param content — The content to write as a string
 * @impure has side effects / drives control flow
 */
declare function writeString({ path: Struct, content: string }): void;


// === Data/Files/Directories ===

/**
 * Converts the cache directory to a Path
 * @param nodeScope (optional) — Is this node in the node scope?
 * @param userScope (optional) — Store in user context?
 * @returns path — Output Path
 */
declare function pathFromCacheDir({ nodeScope?: bool, userScope?: bool }): Struct;

/**
 * Converts the storage directory to a Path
 * @param nodeScope (optional) — Is this node in the node scope?
 * @returns path — Output Path
 */
declare function pathFromStorageDir({ nodeScope?: bool }): Struct;

/**
 * Converts the upload directory to a Path
 * @returns path — Output Path
 */
declare function pathFromUploadDir(): Struct;

/**
 * Converts the user directory to a Path
 * @param nodeScope (optional) — Is this node in the node scope?
 * @returns path — Output Path
 */
declare function pathFromUserDir({ nodeScope?: bool }): Struct;

/**
 * Creates an in-memory virtual directory path
 * @param name (optional) — Virtual directory name
 * @returns path — Virtual directory path
 */
declare function pathVirtualDir({ name?: string }): Struct;


// === Data/Files/External ===

/**
 * Turn an Azure Blob Storage container into a FlowPath. Takes an AzureProvider.
 * @param provider — Azure provider (from the Azure Provider node)
 * @param container — Azure blob container name
 * @param prefix (optional) — Optional path prefix within the container
 * @returns path — FlowPath pointing to the Azure Blob location
 * @impure has side effects / drives control flow
 */
declare function externalAzureBlobStore({ provider: Struct, container: string, prefix?: string }): Struct;

/**
 * Turn a Google Cloud Storage bucket into a FlowPath. Takes a GcpProvider.
 * @param provider — GCP provider (from the GCP Provider node)
 * @param bucket — GCS bucket name
 * @param prefix (optional) — Optional path prefix within the bucket
 * @returns path — FlowPath pointing to the GCS location
 * @impure has side effects / drives control flow
 */
declare function externalGcpStorageStore({ provider: Struct, bucket: string, prefix?: string }): Struct;

/**
 * Turn a Cloudflare R2 bucket into a FlowPath. Takes a CloudflareProvider in 'r2' auth mode (account_id + R2 access key/secret).
 * @param provider — Cloudflare provider (from the Cloudflare Provider node, auth_mode='r2')
 * @param bucket — R2 bucket name
 * @param prefix (optional) — Optional path prefix within the bucket
 * @returns path — FlowPath pointing to the R2 location
 * @impure has side effects / drives control flow
 */
declare function externalR2Store({ provider: Struct, bucket: string, prefix?: string }): Struct;

/**
 * Turn an S3 Express One Zone bucket into a FlowPath. Ultra-low latency single-AZ storage. Takes an AwsProvider.
 * @param provider — AWS provider (from the AWS Provider node)
 * @param bucket — S3 Express bucket name (must end with --azid--x-s3)
 * @param prefix (optional) — Optional path prefix within the bucket
 * @returns path — FlowPath pointing to the S3 Express location
 * @impure has side effects / drives control flow
 */
declare function externalS3ExpressStore({ provider: Struct, bucket: string, prefix?: string }): Struct;

/**
 * Turn an S3 bucket (or any S3-compatible endpoint) into a FlowPath. Takes an AwsProvider for authentication. Use a CloudflareProvider + R2 node for Cloudflare R2 — it's specialised.
 * @param provider — AWS provider (from the AWS Provider node)
 * @param bucket — S3 bucket name
 * @param prefix (optional) — Optional path prefix within the bucket
 * @param pathStyle (optional) — Use path-style URLs (required for some S3-compatible services, e.g. MinIO)
 * @returns path — FlowPath pointing to the S3 location
 * @impure has side effects / drives control flow
 */
declare function externalS3Store({ provider: Struct, bucket: string, prefix?: string, pathStyle?: bool }): Struct;

/**
 * Turn an SMB2/3 share into a FlowPath.
 * @param address — SMB server address. Use host:port, or host to use port 445.
 * @param share — SMB share name
 * @param authMode (optional) — How to authenticate: 'credentials' (username/password/domain), 'guest', or 'kerberos_ccache' (local FILE ccache/kinit)
 * @param prefix (optional) — Optional path prefix within the share
 * @param username (optional) — SMB username
 * @param password (optional) — SMB password
 * @param domain (optional) — Optional SMB domain or workgroup
 * @param timeoutSeconds (optional) — Connection timeout in seconds
 * @param compression (optional) — Enable SMB compression when supported by the server
 * @param dfsEnabled (optional) — Enable DFS referral handling
 * @returns path — FlowPath pointing to the SMB share
 * @impure has side effects / drives control flow
 */
declare function externalSmbStore({ address: string, share: string, authMode?: string, prefix?: string, username?: string, password?: string, domain?: string, timeoutSeconds?: int, compression?: bool, dfsEnabled?: bool }): Struct;


// === Data/Files/Operations ===

/**
 * Checks if a path exists
 * @param path — FlowPath
 * @impure has side effects / drives control flow
 */
declare function pathExists({ path: Struct }): void;

/**
 * Reads all bytes from a file
 * @param path — FlowPath
 * @returns bytes — Output Bytes
 * @impure has side effects / drives control flow
 */
declare function pathGet({ path: Struct }): bytes[];

/**
 * Reads a range of bytes from a file
 * @param path — FlowPath
 * @param from — Start of the Range
 * @param to — End of the Range
 * @returns bytes — Output Bytes
 * @impure has side effects / drives control flow
 */
declare function pathGetRange({ path: Struct, from: int, to: int }): bytes[];

/**
 * Hashes a file
 * @param path — FlowPath
 * @returns hash — Output Hash
 * @impure has side effects / drives control flow
 */
declare function pathHashFile({ path: Struct }): string;

/**
 * Gets the metadata of a file
 * @param path — FlowPath
 * @returns eTag — Etag
 * @returns lastModified — Last Modified
 * @returns size — Size
 * @returns version — Version
 * @impure has side effects / drives control flow
 */
declare function pathHead({ path: Struct }): { eTag: string, lastModified: Date, size: int, version: string };

/**
 * Lists folders under a path
 * @param prefix — FlowPath
 * @param recursive (optional) — List folders recursively
 * @returns folders — Output Folders
 * @impure has side effects / drives control flow
 */
declare function pathListFolders({ prefix: Struct, recursive?: bool }): Struct[];

/**
 * Lists all paths in a directory
 * @param prefix — FlowPath
 * @param recursive (optional) — List paths recursively
 * @returns paths — Output Paths
 * @impure has side effects / drives control flow
 */
declare function pathListPaths({ prefix: Struct, recursive?: bool }): Struct[];

/**
 * Lists paths in a directory with offset and limit
 * @param prefix — FlowPath
 * @param offset — FlowPath
 * @param offset — Offset to start listing from
 * @returns paths — Output Paths
 * @impure has side effects / drives control flow
 */
declare function pathListWithOffset({ prefix: Struct, offset: Struct, offset: int }): Struct[];

/**
 * Writes bytes to a file
 * @param path — FlowPath
 * @param bytes — Bytes to write
 * @impure has side effects / drives control flow
 */
declare function pathPut({ path: Struct, bytes: bytes[] }): void;

/**
 * Renames a file
 * @param from — Source FlowPath
 * @param to — Destination FlowPath
 * @param overwrite (optional) — Should the destination file be overwritten?
 * @impure has side effects / drives control flow
 */
declare function pathRename({ from: Struct, to: Struct, overwrite?: bool }): void;

/**
 * Generates a signed URL for accessing a file
 * @param path — FlowPath
 * @param method (optional) — HTTP Method (GET, PUT, etc.)
 * @param expiration (optional) — Expiration time in seconds for the signed URL
 * @returns signedUrl — The generated signed URL
 * @impure has side effects / drives control flow
 */
declare function signUrl({ path: Struct, method?: string, expiration?: int }): string;

/**
 * Generates signed URLs for accessing files
 * @param paths — Array of FlowPaths
 * @param method (optional) — HTTP Method (GET, PUT, etc.)
 * @param expiration (optional) — Expiration time in seconds for the signed URLs
 * @returns signedUrls — The generated array of signed URLs
 * @impure has side effects / drives control flow
 */
declare function signUrls({ paths: Struct[], method?: string, expiration?: int }): string[];

/**
 * Copies a file from one location to another
 * @param from — Source Path
 * @param to — Destination Path
 * @impure has side effects / drives control flow
 */
declare function storageCopy({ from: Struct, to: Struct }): void;

/**
 * Deletes a file or directory
 * @param path — Path to delete
 * @param recursive (optional) — Delete directories recursively
 * @impure has side effects / drives control flow
 */
declare function storageDelete({ path: Struct, recursive?: bool }): void;


// === Data/Files/Path ===

/**
 * Creates a child path from a parent path
 * @param parentPath — Parent FlowPath
 * @param childName — Name of the child
 * @returns path — Child Path
 */
declare function child({ parentPath: Struct, childName: string }): Struct;

/**
 * Gets the file extension from a path
 * @param path — FlowPath
 * @returns extension — File Extension
 */
declare function extension({ path: Struct }): string;

/**
 * Gets the filename from a path
 * @param path — FlowPath
 * @param removeExtension (optional) — Remove Extension from the Path
 * @returns filename — Filename
 */
declare function filename({ path: Struct, removeExtension?: bool }): string;

/**
 * Reconstructs a FlowPath from a raw path string using the store reference from a base path
 * @param basePath — FlowPath to get the store reference from
 * @param rawPath — The raw path string to reconstruct
 * @returns path — Reconstructed FlowPath
 */
declare function fromRawPath({ basePath: Struct, rawPath: string }): Struct;

/**
 * Gets the parent path from a path
 * @param path — FlowPath
 * @returns parentPath — Parent FlowPath
 * @impure has side effects / drives control flow
 */
declare function parent({ path: Struct }): Struct;

/**
 * Replaces a segment in a FlowPath
 * @param inPath — FlowPath
 * @param from — Segment to replace
 * @param to — Replacement segment
 * @param replaceAll (optional) — Replace all matching segments
 * @returns outPath — Updated FlowPath
 */
declare function pathReplaceSegment({ inPath: Struct, from: string, to: string, replaceAll?: bool }): Struct;

/**
 * Gets the raw path string
 * @param path — FlowPath
 * @returns rawPath — Raw Path String
 */
declare function rawPath({ path: Struct }): string;

/**
 * Sets the file extension of a path
 * @param path — FlowPath
 * @param extension — New File Extension
 * @returns pathOut — Modified FlowPath
 * @impure has side effects / drives control flow
 */
declare function setExtension({ path: Struct, extension: string }): Struct;

/**
 * Gets the filename from a path
 * @param inPath — FlowPath
 * @param filename (optional) — Filename
 * @returns outPath — FlowPath
 */
declare function setFilename({ inPath: Struct, filename?: string }): Struct;


// === Data/GitHub ===

/**
 * Add a comment to an issue or pull request
 * @param provider — GitHub provider
 * @param owner — Repository owner
 * @param repo — Repository name
 * @param issueNumber — Issue or PR number
 * @param body — Comment body (Markdown supported)
 * @returns comment — Created comment
 * @returns commentId — ID of the created comment
 * @impure has side effects / drives control flow
 */
declare function dataGithubAddIssueComment({ provider: Struct, owner: string, repo: string, issueNumber: int, body: string }): { comment: Struct, commentId: int };

/**
 * Clone a GitHub repository. Works with any FlowPath store type (local, S3, memory, etc.). For non-local stores, clones to a temp directory first, then copies files into the target store.
 * @param provider — GitHub provider for authentication
 * @param owner — Repository owner
 * @param repo — Repository name
 * @param targetDir — FlowPath directory to clone into (supports any store type)
 * @param branch (optional) — Branch to clone (leave empty for default branch)
 * @param depth (optional) — Shallow clone depth (0 for full clone)
 * @param includeGit (optional) — Include the .git directory (only useful for local stores)
 * @returns repoPath — FlowPath to the cloned repository
 * @impure has side effects / drives control flow
 */
declare function dataGithubCloneRepo({ provider: Struct, owner: string, repo: string, targetDir: Struct, branch?: string, depth?: int, includeGit?: bool }): Struct;

/**
 * Compare two commits, branches, or tags
 * @param provider — GitHub provider
 * @param owner — Repository owner
 * @param repo — Repository name
 * @param base — Base branch, tag, or SHA
 * @param head — Head branch, tag, or SHA
 * @returns comparison — Comparison result
 * @returns status — Comparison status (ahead, behind, identical, diverged)
 * @returns aheadBy — Number of commits ahead
 * @returns behindBy — Number of commits behind
 * @impure has side effects / drives control flow
 */
declare function dataGithubCompareCommits({ provider: Struct, owner: string, repo: string, base: string, head: string }): { comparison: Struct, status: string, aheadBy: int, behindBy: int };

/**
 * Create a new branch from a reference (branch name or SHA)
 * @param provider — GitHub provider
 * @param owner — Repository owner
 * @param repo — Repository name
 * @param branch — Name for the new branch
 * @param fromSha — SHA to create branch from (get from Get Branch node)
 * @returns ref — Created reference (refs/heads/...)
 * @impure has side effects / drives control flow
 */
declare function dataGithubCreateBranch({ provider: Struct, owner: string, repo: string, branch: string, fromSha: string }): string;

/**
 * Create a new issue in a repository
 * @param provider — GitHub provider
 * @param owner — Repository owner
 * @param repo — Repository name
 * @param title — Issue title
 * @param body (optional) — Issue body (Markdown supported)
 * @param labels — Label names to apply
 * @param assignees — Usernames to assign
 * @param milestone (optional) — Milestone number to associate with the issue
 * @param issueType (optional) — Issue type name or ID
 * @param issueFieldValues — Issue form field values accepted by the GitHub API
 * @returns issue — Created issue
 * @returns issueNumber — The number of the created issue
 * @impure has side effects / drives control flow
 */
declare function dataGithubCreateIssue({ provider: Struct, owner: string, repo: string, title: string, body?: string, labels: string[], assignees: string[], milestone?: int, issueType?: string, issueFieldValues: Struct[] }): { issue: Struct, issueNumber: int };

/**
 * Create or update a file in a repository
 * @param provider — GitHub provider
 * @param owner — Repository owner
 * @param repo — Repository name
 * @param path — Path to file in the repository
 * @param sourceFile — FlowPath file to upload. When connected, this is used instead of Content
 * @param content (optional) — Text content used when Source File is not connected
 * @param message — Commit message
 * @param sha (optional) — SHA of the file being replaced (required for updates, get from 'Get File Contents')
 * @param branch (optional) — Branch to commit to (default: default branch)
 * @param committerName (optional) — Name of the committer (default: authenticated user)
 * @param committerEmail (optional) — Email of the committer (default: authenticated user's email)
 * @param authorName (optional) — Name of the author (default: authenticated user)
 * @param authorEmail (optional) — Email of the author (default: authenticated user's email)
 * @returns commitSha — SHA of the commit
 * @returns fileSha — New SHA of the file
 * @impure has side effects / drives control flow
 */
declare function dataGithubCreateOrUpdateFile({ provider: Struct, owner: string, repo: string, path: string, sourceFile: Struct, content?: string, message: string, sha?: string, branch?: string, committerName?: string, committerEmail?: string, authorName?: string, authorEmail?: string }): { commitSha: string, fileSha: string };

/**
 * Create a review on a pull request
 * @param provider — GitHub provider
 * @param owner — Repository owner
 * @param repo — Repository name
 * @param prNumber — Pull request number
 * @param body (optional) — Review comment body
 * @param commitId (optional) — SHA of the commit needing a review
 * @param comments — Inline review comments with path, position or line, and body
 * @param event (optional) — Review event type
 * @returns review — Created review
 * @returns reviewId — ID of the created review
 * @impure has side effects / drives control flow
 */
declare function dataGithubCreatePrReview({ provider: Struct, owner: string, repo: string, prNumber: int, body?: string, commitId?: string, comments: Struct[], event?: string }): { review: Struct, reviewId: int };

/**
 * Create a new pull request
 * @param provider — GitHub provider
 * @param owner — Repository owner
 * @param repo — Repository name
 * @param title (optional) — Pull request title
 * @param head — Branch containing changes (owner:branch for cross-repo)
 * @param base — Branch to merge into
 * @param issue (optional) — Issue number to convert into a pull request
 * @param headRepo (optional) — Repository name containing the head branch when both repos are owned by the same organization
 * @param body (optional) — Pull request description (Markdown supported)
 * @param draft (optional) — Create as draft pull request
 * @param maintainerCanModify (optional) — Allow maintainers to modify the PR
 * @returns pullRequest — Created pull request
 * @returns prNumber — Pull request number
 * @returns htmlUrl — Pull request URL
 * @impure has side effects / drives control flow
 */
declare function dataGithubCreatePullRequest({ provider: Struct, owner: string, repo: string, title?: string, head: string, base: string, issue?: int, headRepo?: string, body?: string, draft?: bool, maintainerCanModify?: bool }): { pullRequest: Struct, prNumber: int, htmlUrl: string };

/**
 * Create a new release
 * @param provider — GitHub provider
 * @param owner — Repository owner
 * @param repo — Repository name
 * @param tagName — Tag name for the release (e.g., v1.0.0)
 * @param name (optional) — Release title (defaults to tag name if empty)
 * @param body (optional) — Release notes (Markdown supported)
 * @param targetCommitish (optional) — Branch or SHA to tag (default: default branch)
 * @param draft (optional) — Create as draft release
 * @param prerelease (optional) — Mark as prerelease
 * @param generateReleaseNotes (optional) — Auto-generate release notes from commits
 * @param discussionCategoryName (optional) — Discussion category name to link a discussion to the release
 * @param makeLatest (optional) — Controls whether this release is the latest release
 * @returns release — Created release
 * @returns releaseId — ID of the created release
 * @returns htmlUrl — URL to the release
 * @impure has side effects / drives control flow
 */
declare function dataGithubCreateRelease({ provider: Struct, owner: string, repo: string, tagName: string, name?: string, body?: string, targetCommitish?: string, draft?: bool, prerelease?: bool, generateReleaseNotes?: bool, discussionCategoryName?: string, makeLatest?: string }): { release: Struct, releaseId: int, htmlUrl: string };

/**
 * Delete a branch from a repository
 * @param provider — GitHub provider
 * @param owner — Repository owner
 * @param repo — Repository name
 * @param branch — Branch to delete
 * @impure has side effects / drives control flow
 */
declare function dataGithubDeleteBranch({ provider: Struct, owner: string, repo: string, branch: string }): void;

/**
 * Delete a file from a repository
 * @param provider — GitHub provider
 * @param owner — Repository owner
 * @param repo — Repository name
 * @param path — Path to file in the repository
 * @param sha — SHA of the file to delete (get from 'Get File Contents')
 * @param message — Commit message
 * @param branch (optional) — Branch to delete from (default: default branch)
 * @param committerName (optional) — Name of the committer (default: authenticated user)
 * @param committerEmail (optional) — Email of the committer (default: authenticated user's email)
 * @param authorName (optional) — Name of the author (default: authenticated user)
 * @param authorEmail (optional) — Email of the author (default: authenticated user's email)
 * @returns commitSha — SHA of the delete commit
 * @impure has side effects / drives control flow
 */
declare function dataGithubDeleteFile({ provider: Struct, owner: string, repo: string, path: string, sha: string, message: string, branch?: string, committerName?: string, committerEmail?: string, authorName?: string, authorEmail?: string }): string;

/**
 * Download raw file content from a repository (for large files)
 * @param provider — GitHub provider
 * @param owner — Repository owner
 * @param repo — Repository name
 * @param path — Path to file in the repository
 * @param ref (optional) — Branch, tag, or commit SHA (default: default branch)
 * @param outputPath — FlowPath to write the downloaded file into
 * @returns writtenPath — Written file path
 * @returns size — File size in bytes
 * @impure has side effects / drives control flow
 */
declare function dataGithubDownloadFile({ provider: Struct, owner: string, repo: string, path: string, ref?: string, outputPath: Struct }): { writtenPath: Struct, size: int };

/**
 * Download a release asset into a FlowPath
 * @param provider — GitHub provider
 * @param owner — Repository owner
 * @param repo — Repository name
 * @param assetId — Release asset ID
 * @param outputPath — FlowPath to write the downloaded asset into
 * @returns path — Written file path
 * @returns size — File size in bytes
 * @impure has side effects / drives control flow
 */
declare function dataGithubDownloadReleaseAsset({ provider: Struct, owner: string, repo: string, assetId: int, outputPath: Struct }): { path: Struct, size: int };

/**
 * Get details about a specific branch
 * @param provider — GitHub provider
 * @param owner — Repository owner
 * @param repo — Repository name
 * @param branch — Branch name
 * @returns branchInfo — Branch information
 * @returns sha — Latest commit SHA
 * @impure has side effects / drives control flow
 */
declare function dataGithubGetBranch({ provider: Struct, owner: string, repo: string, branch: string }): { branchInfo: Struct, sha: string };

/**
 * Get details about a specific commit
 * @param provider — GitHub provider
 * @param owner — Repository owner
 * @param repo — Repository name
 * @param sha — Commit SHA, branch, or tag
 * @returns commit — Commit information
 * @returns message — Commit message
 * @returns additions — Lines added
 * @returns deletions — Lines deleted
 * @impure has side effects / drives control flow
 */
declare function dataGithubGetCommit({ provider: Struct, owner: string, repo: string, sha: string }): { commit: Struct, message: string, additions: int, deletions: int };

/**
 * Get the contents of a file from a repository
 * @param provider — GitHub provider
 * @param owner — Repository owner
 * @param repo — Repository name
 * @param path — Path to file in the repository
 * @param ref (optional) — Branch, tag, or commit SHA (default: default branch)
 * @returns fileInfo — File metadata
 * @returns content — Decoded UTF-8 file content
 * @returns base64Content — Exact file content returned by GitHub, without line breaks
 * @returns sha — File SHA (needed for updates)
 * @impure has side effects / drives control flow
 */
declare function dataGithubGetFileContents({ provider: Struct, owner: string, repo: string, path: string, ref?: string }): { fileInfo: Struct, content: string, base64Content: string, sha: string };

/**
 * Get details about a specific issue
 * @param provider — GitHub provider
 * @param owner — Repository owner
 * @param repo — Repository name
 * @param issueNumber — Issue number
 * @returns issue — Issue details
 * @returns title — Issue title
 * @returns body — Issue body
 * @returns state — Issue state (open/closed)
 * @impure has side effects / drives control flow
 */
declare function dataGithubGetIssue({ provider: Struct, owner: string, repo: string, issueNumber: int }): { issue: Struct, title: string, body: string, state: string };

/**
 * Get the latest published release (excludes drafts and prereleases)
 * @param provider — GitHub provider
 * @param owner — Repository owner
 * @param repo — Repository name
 * @returns release — Latest release
 * @returns tagName — Release tag name
 * @returns name — Release name
 * @impure has side effects / drives control flow
 */
declare function dataGithubGetLatestRelease({ provider: Struct, owner: string, repo: string }): { release: Struct, tagName: string, name: string };

/**
 * Get details about a specific pull request
 * @param provider — GitHub provider
 * @param owner — Repository owner
 * @param repo — Repository name
 * @param prNumber — Pull request number
 * @returns pullRequest — Pull request details
 * @returns title — PR title
 * @returns body — PR body
 * @returns state — PR state (open/closed)
 * @returns mergeable — Whether the PR can be merged
 * @impure has side effects / drives control flow
 */
declare function dataGithubGetPullRequest({ provider: Struct, owner: string, repo: string, prNumber: int }): { pullRequest: Struct, title: string, body: string, state: string, mergeable: bool };

/**
 * Get a release by its tag name
 * @param provider — GitHub provider
 * @param owner — Repository owner
 * @param repo — Repository name
 * @param tag — Tag name (e.g., v1.0.0)
 * @returns release — Release details
 * @returns body — Release notes
 * @impure has side effects / drives control flow
 */
declare function dataGithubGetReleaseByTag({ provider: Struct, owner: string, repo: string, tag: string }): { release: Struct, body: string };

/**
 * Get detailed information about a specific repository
 * @param provider — GitHub provider
 * @param owner — Repository owner (user or organization)
 * @param repo — Repository name
 * @returns repository — Repository details
 * @impure has side effects / drives control flow
 */
declare function dataGithubGetRepo({ provider: Struct, owner: string, repo: string }): Struct;

/**
 * Get information about a GitHub user, or the authenticated user if no username provided
 * @param provider — GitHub provider
 * @param username (optional) — GitHub username. Leave empty to get authenticated user
 * @returns user — User details
 * @impure has side effects / drives control flow
 */
declare function dataGithubGetUser({ provider: Struct, username?: string }): Struct;

/**
 * List branches for a repository
 * @param provider — GitHub provider
 * @param owner — Repository owner
 * @param repo — Repository name
 * @param protectedOnly (optional) — Only list protected branches
 * @param perPage (optional) — Results per page (max 100)
 * @param page (optional) — Page number
 * @returns branches — Array of branches
 * @returns count — Number of branches returned
 * @impure has side effects / drives control flow
 */
declare function dataGithubListBranches({ provider: Struct, owner: string, repo: string, protectedOnly?: bool, perPage?: int, page?: int }): { branches: Struct[], count: int };

/**
 * List commits for a repository
 * @param provider — GitHub provider
 * @param owner — Repository owner
 * @param repo — Repository name
 * @param sha (optional) — SHA or branch name to start listing commits from
 * @param path (optional) — Only commits containing this file path
 * @param author (optional) — GitHub username or email to filter by
 * @param committer (optional) — GitHub username or email to filter by committer
 * @param since (optional) — Only commits after this date (ISO 8601 format)
 * @param until (optional) — Only commits before this date (ISO 8601 format)
 * @param perPage (optional) — Results per page (max 100)
 * @param page (optional) — Page number
 * @returns commits — Array of commits
 * @returns count — Number of commits returned
 * @impure has side effects / drives control flow
 */
declare function dataGithubListCommits({ provider: Struct, owner: string, repo: string, sha?: string, path?: string, author?: string, committer?: string, since?: string, until?: string, perPage?: int, page?: int }): { commits: Struct[], count: int };

/**
 * List comments on an issue or pull request
 * @param provider — GitHub provider
 * @param owner — Repository owner
 * @param repo — Repository name
 * @param issueNumber — Issue or PR number
 * @param perPage (optional) — Results per page (max 100)
 * @param page (optional) — Page number
 * @returns comments — Array of comments
 * @returns count — Number of comments returned
 * @impure has side effects / drives control flow
 */
declare function dataGithubListIssueComments({ provider: Struct, owner: string, repo: string, issueNumber: int, perPage?: int, page?: int }): { comments: Struct[], count: int };

/**
 * List issues for a repository
 * @param provider — GitHub provider
 * @param owner — Repository owner
 * @param repo — Repository name
 * @param state (optional) — Issue state: open, closed, all
 * @param milestone (optional) — Milestone number, none, or *
 * @param labels (optional) — Comma-separated list of label names
 * @param assignee (optional) — Filter by assignee username. Use * for any, none for no assignee
 * @param creator (optional) — Filter by issue creator username
 * @param mentioned (optional) — Filter by mentioned username
 * @param issueType (optional) — Filter by issue type
 * @param issueFieldValues (optional) — Filter by issue field values
 * @param sort (optional) — Sort field
 * @param direction (optional) — Sort direction
 * @param perPage (optional) — Results per page (max 100)
 * @param page (optional) — Page number
 * @returns issues — Array of issues
 * @returns count — Number of issues returned
 * @impure has side effects / drives control flow
 */
declare function dataGithubListIssues({ provider: Struct, owner: string, repo: string, state?: string, milestone?: string, labels?: string, assignee?: string, creator?: string, mentioned?: string, issueType?: string, issueFieldValues?: string, sort?: string, direction?: string, perPage?: int, page?: int }): { issues: Struct[], count: int };

/**
 * List files changed in a pull request
 * @param provider — GitHub provider
 * @param owner — Repository owner
 * @param repo — Repository name
 * @param prNumber — Pull request number
 * @param perPage (optional) — Results per page (max 100)
 * @param page (optional) — Page number
 * @returns files — Array of changed files
 * @returns count — Number of files changed
 * @returns additions — Total lines added
 * @returns deletions — Total lines deleted
 * @impure has side effects / drives control flow
 */
declare function dataGithubListPrFiles({ provider: Struct, owner: string, repo: string, prNumber: int, perPage?: int, page?: int }): { files: Struct[], count: int, additions: int, deletions: int };

/**
 * List reviews on a pull request
 * @param provider — GitHub provider
 * @param owner — Repository owner
 * @param repo — Repository name
 * @param prNumber — Pull request number
 * @param perPage (optional) — Results per page (max 100)
 * @param page (optional) — Page number
 * @returns reviews — Array of reviews
 * @returns count — Number of reviews
 * @impure has side effects / drives control flow
 */
declare function dataGithubListPrReviews({ provider: Struct, owner: string, repo: string, prNumber: int, perPage?: int, page?: int }): { reviews: Struct[], count: int };

/**
 * List pull requests for a repository
 * @param provider — GitHub provider
 * @param owner — Repository owner
 * @param repo — Repository name
 * @param state (optional) — PR state: open, closed, all
 * @param head (optional) — Filter by head user or head user:ref (e.g., user:branch-name)
 * @param base (optional) — Filter by base branch name
 * @param sort (optional) — Sort by: created, updated, popularity, long-running
 * @param direction (optional) — Sort direction: asc, desc
 * @param perPage (optional) — Results per page (max 100)
 * @param page (optional) — Page number
 * @returns pullRequests — Array of pull requests
 * @returns count — Number of pull requests returned
 * @impure has side effects / drives control flow
 */
declare function dataGithubListPullRequests({ provider: Struct, owner: string, repo: string, state?: string, head?: string, base?: string, sort?: string, direction?: string, perPage?: int, page?: int }): { pullRequests: Struct[], count: int };

/**
 * List assets attached to a release
 * @param provider — GitHub provider
 * @param owner — Repository owner
 * @param repo — Repository name
 * @param releaseId — Release ID
 * @param perPage (optional) — Results per page (max 100)
 * @param page (optional) — Page number
 * @returns assets — Release assets
 * @returns count — Number of assets
 * @impure has side effects / drives control flow
 */
declare function dataGithubListReleaseAssets({ provider: Struct, owner: string, repo: string, releaseId: int, perPage?: int, page?: int }): { assets: Struct[], count: int };

/**
 * List releases for a repository
 * @param provider — GitHub provider
 * @param owner — Repository owner
 * @param repo — Repository name
 * @param perPage (optional) — Results per page (max 100)
 * @param page (optional) — Page number
 * @returns releases — Array of releases
 * @returns count — Number of releases returned
 * @impure has side effects / drives control flow
 */
declare function dataGithubListReleases({ provider: Struct, owner: string, repo: string, perPage?: int, page?: int }): { releases: Struct[], count: int };

/**
 * List repositories for the authenticated user or a specified organization
 * @param provider — GitHub provider
 * @param org (optional) — Optional organization name. If empty, lists user's repos
 * @param type (optional) — Type of repositories to list
 * @param sort (optional) — Sort field
 * @param direction (optional) — Sort direction
 * @param perPage (optional) — Results per page (max 100)
 * @param page (optional) — Page number
 * @returns repos — Array of repositories
 * @returns count — Number of repos returned
 * @impure has side effects / drives control flow
 */
declare function dataGithubListRepos({ provider: Struct, org?: string, type?: string, sort?: string, direction?: string, perPage?: int, page?: int }): { repos: Struct[], count: int };

/**
 * Merge a pull request
 * @param provider — GitHub provider
 * @param owner — Repository owner
 * @param repo — Repository name
 * @param prNumber — Pull request number
 * @param commitTitle (optional) — Title for the merge commit (leave empty for default)
 * @param sha (optional) — Expected SHA of the pull request head to guard against merging stale changes
 * @param commitMessage (optional) — Extra detail for merge commit (leave empty for default)
 * @param mergeMethod (optional) — Method to use for merging
 * @returns mergeSha — SHA of merge commit
 * @returns merged — Whether the PR was merged
 * @impure has side effects / drives control flow
 */
declare function dataGithubMergePullRequest({ provider: Struct, owner: string, repo: string, prNumber: int, commitTitle?: string, sha?: string, commitMessage?: string, mergeMethod?: string }): { mergeSha: string, merged: bool };

/**
 * Connect to GitHub using a GitHub App installation token. Use this for server-to-server authentication.
 * @param installationToken — GitHub App installation access token
 * @param baseUrl (optional) — GitHub API base URL. Use 'https://api.github.com' for github.com or 'https://your-enterprise.com/api/v3' for Enterprise
 * @returns provider — GitHub provider with authentication
 */
declare function dataGithubProviderApp({ installationToken: string, baseUrl?: string }): Struct;

/**
 * Connect to GitHub using OAuth Device Flow.
 * @param baseUrl (optional) — GitHub API base URL. Use 'https://api.github.com' for github.com or 'https://your-enterprise.com/api/v3' for Enterprise
 * @returns provider — GitHub provider with authentication
 */
declare function dataGithubProviderOauth({ baseUrl?: string }): Struct;

/**
 * Connect to GitHub using a Personal Access Token. Generate one at github.com/settings/tokens
 * @param token — Your GitHub Personal Access Token (classic or fine-grained)
 * @param baseUrl (optional) — GitHub API base URL. Use 'https://api.github.com' for github.com or 'https://your-enterprise.com/api/v3' for Enterprise
 * @returns provider — GitHub provider with authentication
 */
declare function dataGithubProviderPat({ token: string, baseUrl?: string }): Struct;

/**
 * Search for code across GitHub repositories
 * @param provider — GitHub provider
 * @param query — Search query. Use GitHub code search syntax
 * @param repo (optional) — Limit to a specific repo (owner/repo format)
 * @param language (optional) — Filter by programming language
 * @param path (optional) — Filter by file path
 * @param extension (optional) — Filter by file extension (e.g., rs, ts, py)
 * @param sort (optional) — Sort field
 * @param order (optional) — Sort order
 * @param perPage (optional) — Results per page (max 100)
 * @param page (optional) — Page number
 * @returns results — Array of code search results
 * @returns totalCount — Total number of matching results (may be > returned)
 * @impure has side effects / drives control flow
 */
declare function dataGithubSearchCode({ provider: Struct, query: string, repo?: string, language?: string, path?: string, extension?: string, sort?: string, order?: string, perPage?: int, page?: int }): { results: Struct[], totalCount: int };

/**
 * Search for issues across GitHub repositories
 * @param provider — GitHub provider
 * @param owner (optional) — Repository owner (optional)
 * @param repo (optional) — Repository name (optional)
 * @param query — Search query. Use GitHub search syntax
 * @param state (optional) — Filter by state: open, closed
 * @param type (optional) — Filter by type: issue, pr
 * @param author (optional) — Filter by author username
 * @param assignee (optional) — Filter by assignee username
 * @param labels (optional) — Filter by labels (comma-separated)
 * @param sort (optional) — Sort by: comments, reactions, reactions-+1, interactions, created, updated
 * @param order (optional) — Sort order: asc, desc
 * @param perPage (optional) — Results per page (max 100)
 * @param page (optional) — Page number
 * @returns issues — Array of matching issues
 * @returns totalCount — Total number of matching issues
 * @impure has side effects / drives control flow
 */
declare function dataGithubSearchIssues({ provider: Struct, owner?: string, repo?: string, query: string, state?: string, type?: string, author?: string, assignee?: string, labels?: string, sort?: string, order?: string, perPage?: int, page?: int }): { issues: Struct[], totalCount: int };

/**
 * Search for repositories on GitHub
 * @param provider — GitHub provider
 * @param query — Search query. Use GitHub search syntax
 * @param language (optional) — Filter by programming language
 * @param user (optional) — Filter by user or organization
 * @param topic (optional) — Filter by topic
 * @param sort (optional) — Sort by: stars, forks, help-wanted-issues, updated
 * @param order (optional) — Sort order: asc, desc
 * @param perPage (optional) — Results per page (max 100)
 * @param page (optional) — Page number
 * @returns repos — Array of repositories
 * @returns totalCount — Total number of matching repositories
 * @impure has side effects / drives control flow
 */
declare function dataGithubSearchRepos({ provider: Struct, query: string, language?: string, user?: string, topic?: string, sort?: string, order?: string, perPage?: int, page?: int }): { repos: Struct[], totalCount: int };

/**
 * Update an existing issue
 * @param provider — GitHub provider
 * @param owner — Repository owner
 * @param repo — Repository name
 * @param issueNumber — Issue number to update
 * @param title (optional) — New title (leave empty to keep current)
 * @param body (optional) — New body (leave empty to keep current)
 * @param state (optional) — New state: open or closed (leave empty to keep current)
 * @param stateReason (optional) — Reason for the state change
 * @param labels — Label names (replaces all labels when connected)
 * @param assignees — Usernames (replaces all assignees when connected)
 * @param milestone (optional) — Milestone number to associate with the issue
 * @param issueType (optional) — Issue type name or ID
 * @param issueFieldValues — Issue form field values accepted by the GitHub API
 * @returns issue — Updated issue
 * @impure has side effects / drives control flow
 */
declare function dataGithubUpdateIssue({ provider: Struct, owner: string, repo: string, issueNumber: int, title?: string, body?: string, state?: string, stateReason?: string, labels: string[], assignees: string[], milestone?: int, issueType?: string, issueFieldValues: Struct[] }): Struct;

/**
 * Update an existing pull request
 * @param provider — GitHub provider
 * @param owner — Repository owner
 * @param repo — Repository name
 * @param prNumber — Pull request number
 * @param title (optional) — New title (leave empty to keep current)
 * @param body (optional) — New body (leave empty to keep current)
 * @param state (optional) — New state
 * @param base (optional) — New base branch (leave empty to keep current)
 * @param maintainerCanModify — Allow maintainers to modify the PR when connected
 * @returns pullRequest — Updated pull request
 * @impure has side effects / drives control flow
 */
declare function dataGithubUpdatePullRequest({ provider: Struct, owner: string, repo: string, prNumber: int, title?: string, body?: string, state?: string, base?: string, maintainerCanModify: bool }): Struct;

/**
 * Upload a FlowPath file as a release asset
 * @param provider — GitHub provider
 * @param owner — Repository owner
 * @param repo — Repository name
 * @param releaseId — Release ID
 * @param file — FlowPath file to upload
 * @param name (optional) — Asset file name. Uses the FlowPath file name when empty
 * @param label (optional) — Asset label
 * @param contentType (optional) — Asset MIME type
 * @returns asset — Uploaded asset
 * @returns assetId — Uploaded asset ID
 * @returns downloadUrl — Browser download URL
 * @impure has side effects / drives control flow
 */
declare function dataGithubUploadReleaseAsset({ provider: Struct, owner: string, repo: string, releaseId: int, file: Struct, name?: string, label?: string, contentType?: string }): { asset: Struct, assetId: int, downloadUrl: string };


// === Data/GitHub/Workflows ===

/**
 * Cancel a workflow run that is in progress
 * @param provider — GitHub provider with authentication
 * @param owner — Repository owner
 * @param repo — Repository name
 * @param runId — Workflow run ID to cancel
 * @returns errorMessage — Error message if request failed
 * @impure has side effects / drives control flow
 */
declare function githubCancelWorkflowRun({ provider: Struct, owner: string, repo: string, runId: int }): string;

/**
 * Get the most recent workflow run, optionally filtered by conclusion (success/failure)
 * @param provider — GitHub provider with authentication
 * @param owner — Repository owner
 * @param repo — Repository name
 * @param workflowId — Workflow file name or ID (e.g., 'ci.yml')
 * @param branch (optional) — Filter by branch name (optional)
 * @param actor (optional) — Filter by username that triggered the run
 * @param event (optional) — Filter by event type
 * @param created (optional) — Filter runs by creation date range, e.g. >=2024-01-01
 * @param checkSuiteId (optional) — Filter by check suite ID
 * @param headSha (optional) — Filter by head commit SHA
 * @param conclusion (optional) — Filter by conclusion (empty = any, success, failure, cancelled, skipped)
 * @param excludePullRequests (optional) — Exclude runs triggered by pull requests
 * @param perPage (optional) — Number of runs to inspect (max 100)
 * @returns run — The latest workflow run
 * @returns runId — The workflow run ID
 * @returns runNumber — The workflow run number
 * @returns status — Run status (queued, in_progress, completed)
 * @returns runConclusion — Run conclusion (success, failure, etc.)
 * @returns headBranch — The branch the run was triggered on
 * @returns runHeadSha — The commit SHA
 * @returns htmlUrl — Link to the workflow run
 * @returns isSuccess — Whether the run completed successfully
 * @returns errorMessage — Error message if request failed
 * @impure has side effects / drives control flow
 */
declare function githubGetLatestWorkflowRun({ provider: Struct, owner: string, repo: string, workflowId: string, branch?: string, actor?: string, event?: string, created?: string, checkSuiteId?: string, headSha?: string, conclusion?: string, excludePullRequests?: bool, perPage?: int }): { run: Struct, runId: int, runNumber: int, status: string, runConclusion: string, headBranch: string, runHeadSha: string, htmlUrl: string, isSuccess: bool, errorMessage: string };

/**
 * Get details of a specific workflow run
 * @param provider — GitHub provider with authentication
 * @param owner — Repository owner
 * @param repo — Repository name
 * @param runId — Workflow run ID
 * @returns run — Workflow run details
 * @returns errorMessage — Error message if request failed
 * @impure has side effects / drives control flow
 */
declare function githubGetWorkflowRun({ provider: Struct, owner: string, repo: string, runId: int }): { run: Struct, errorMessage: string };

/**
 * List runs for a specific workflow or all workflows in a repository
 * @param provider — GitHub provider with authentication
 * @param owner — Repository owner
 * @param repo — Repository name
 * @param workflowId (optional) — Optional workflow file name or ID to filter runs
 * @param branch (optional) — Filter by branch name
 * @param actor (optional) — Filter by username that triggered the run
 * @param event (optional) — Filter by event type
 * @param status (optional) — Filter by status
 * @param created (optional) — Filter runs by creation date range, e.g. >=2024-01-01
 * @param excludePullRequests (optional) — Exclude runs triggered by pull requests
 * @param checkSuiteId (optional) — Filter by check suite ID
 * @param headSha (optional) — Filter by head commit SHA
 * @param perPage (optional) — Results per page (max 100)
 * @param page (optional) — Page number
 * @returns runs — List of workflow runs
 * @returns errorMessage — Error message if request failed
 * @impure has side effects / drives control flow
 */
declare function githubListWorkflowRuns({ provider: Struct, owner: string, repo: string, workflowId?: string, branch?: string, actor?: string, event?: string, status?: string, created?: string, excludePullRequests?: bool, checkSuiteId?: string, headSha?: string, perPage?: int, page?: int }): { runs: Struct[], errorMessage: string };

/**
 * List GitHub Actions workflows in a repository
 * @param provider — GitHub provider with authentication
 * @param owner — Repository owner
 * @param repo — Repository name
 * @param perPage (optional) — Results per page (max 100)
 * @param page (optional) — Page number
 * @returns workflows — List of workflows
 * @returns errorMessage — Error message if request failed
 * @impure has side effects / drives control flow
 */
declare function githubListWorkflows({ provider: Struct, owner: string, repo: string, perPage?: int, page?: int }): { workflows: Struct[], errorMessage: string };

/**
 * Re-run a workflow run
 * @param provider — GitHub provider with authentication
 * @param owner — Repository owner
 * @param repo — Repository name
 * @param runId — Workflow run ID to rerun
 * @param failedOnly (optional) — Only rerun failed jobs
 * @param enableDebugLogging (optional) — Enable debug logging for the rerun
 * @returns errorMessage — Error message if request failed
 * @impure has side effects / drives control flow
 */
declare function githubRerunWorkflow({ provider: Struct, owner: string, repo: string, runId: int, failedOnly?: bool, enableDebugLogging?: bool }): string;

/**
 * Trigger a GitHub Actions workflow dispatch event
 * @param provider — GitHub provider with authentication
 * @param owner — Repository owner
 * @param repo — Repository name
 * @param workflowId — Workflow file name or ID (e.g., 'main.yml' or workflow ID)
 * @param ref (optional) — Git reference (branch or tag) to run the workflow on
 * @param inputs (optional) — Workflow inputs as JSON object
 * @param returnRunDetails (optional) — Ask GitHub to return the created workflow run when supported
 * @returns run — Created workflow run when returned by GitHub
 * @returns errorMessage — Error message if request failed
 * @impure has side effects / drives control flow
 */
declare function githubTriggerWorkflow({ provider: Struct, owner: string, repo: string, workflowId: string, ref?: string, inputs?: Struct, returnRunDetails?: bool }): { run: Struct, errorMessage: string };


// === Data/Google ===

/**
 * Authenticate with Google to access Drive, Sheets, Docs, Gmail, YouTube, Calendar and more.
 * @returns provider — Google provider with authentication token - works with all Google services
 */
declare function dataGoogleProvider(): Struct;


// === Data/Google/Calendar ===

/**
 * Create a new calendar event
 * @param provider — Google provider
 * @param calendarId (optional) — Calendar ID
 * @param summary — Event title
 * @param description (optional) — Event description
 * @param location (optional) — Event location
 * @param startTime — Start time
 * @param endTime — End time
 * @param timeZone (optional) — Time zone (e.g., America/New_York)
 * @param attendees (optional) — Comma-separated email addresses
 * @param addMeet (optional) — Add Google Meet conference
 * @returns event — Created event
 * @returns eventId
 * @returns meetLink — Google Meet link (if created)
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataGoogleCalendarCreateEvent({ provider: Struct, calendarId?: string, summary: string, description?: string, location?: string, startTime: Date, endTime: Date, timeZone?: string, attendees?: string, addMeet?: bool }): { event: Struct, eventId: string, meetLink: string, errorMessage: string };

/**
 * Delete a calendar event
 * @param provider — Google provider
 * @param calendarId (optional) — Calendar ID
 * @param eventId — Event ID to delete
 * @param sendNotifications (optional) — Notify attendees
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataGoogleCalendarDeleteEvent({ provider: Struct, calendarId?: string, eventId: string, sendNotifications?: bool }): string;

/**
 * Query free/busy information for calendars
 * @param provider — Google provider
 * @param timeMin — Start time (RFC3339)
 * @param timeMax — End time (RFC3339)
 * @param calendarIds (optional) — Comma-separated calendar IDs (default: primary)
 * @returns busyTimes — Busy time slots per calendar
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataGoogleCalendarFreeBusy({ provider: Struct, timeMin: string, timeMax: string, calendarIds?: string }): { busyTimes: any, errorMessage: string };

/**
 * Get a specific calendar event
 * @param provider — Google provider
 * @param calendarId (optional) — Calendar ID
 * @param eventId — Event ID
 * @returns event — Event details
 * @returns raw — Raw API response
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataGoogleCalendarGetEvent({ provider: Struct, calendarId?: string, eventId: string }): { event: Struct, raw: any, errorMessage: string };

/**
 * List all Google Calendars
 * @param provider — Google provider
 * @returns calendars — List of calendars
 * @returns primaryCalendarId
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataGoogleCalendarList({ provider: Struct }): { calendars: Struct[], primaryCalendarId: string, errorMessage: string };

/**
 * List events from a Google Calendar
 * @param provider — Google provider
 * @param calendarId (optional) — Calendar ID (default: primary)
 * @param timeMin (optional) — Start time (RFC3339, e.g., 2024-01-01T00:00:00Z)
 * @param timeMax (optional) — End time (RFC3339)
 * @param maxResults (optional) — Maximum results (1-2500)
 * @param pageToken (optional) — Token for pagination
 * @returns events — List of events
 * @returns nextPageToken
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataGoogleCalendarListEvents({ provider: Struct, calendarId?: string, timeMin?: string, timeMax?: string, maxResults?: int, pageToken?: string }): { events: Struct[], nextPageToken: string, errorMessage: string };

/**
 * Create an event from natural language text
 * @param provider — Google provider
 * @param calendarId (optional) — Calendar ID
 * @param text — Natural language event description (e.g., 'Lunch with John tomorrow at noon')
 * @returns event — Created event
 * @returns eventId
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataGoogleCalendarQuickAdd({ provider: Struct, calendarId?: string, text: string }): { event: Struct, eventId: string, errorMessage: string };

/**
 * Update an existing calendar event
 * @param provider — Google provider
 * @param calendarId (optional) — Calendar ID
 * @param eventId — Event ID to update
 * @param summary (optional) — Event title (empty to keep)
 * @param description (optional) — Event description (empty to keep)
 * @param location (optional) — Event location (empty to keep)
 * @param startTime — Start time (leave empty to keep)
 * @param endTime — End time (leave empty to keep)
 * @returns event — Updated event
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataGoogleCalendarUpdateEvent({ provider: Struct, calendarId?: string, eventId: string, summary?: string, description?: string, location?: string, startTime: Date, endTime: Date }): { event: Struct, errorMessage: string };


// === Data/Google/Docs ===

/**
 * Create a new Google Document
 * @param provider — Google Drive provider
 * @param title — Document title
 * @returns documentId
 * @returns document
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataGoogleDocsCreate({ provider: Struct, title: string }): { documentId: string, document: Struct, errorMessage: string };

/**
 * Delete text from a range in a Google Document
 * @param provider — Google Drive provider
 * @param documentId
 * @param startIndex — Start index (inclusive)
 * @param endIndex — End index (exclusive)
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataGoogleDocsDeleteText({ provider: Struct, documentId: string, startIndex: int, endIndex: int }): string;

/**
 * Export a Google Document into a FlowPath
 * @param provider — Google Drive provider
 * @param documentId
 * @param format (optional) — Export format
 * @param outputPath — FlowPath to write the exported document into
 * @returns path — Written file path
 * @returns size — Exported size in bytes
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataGoogleDocsExport({ provider: Struct, documentId: string, format?: string, outputPath: Struct }): { path: Struct, size: int, errorMessage: string };

/**
 * Get a Google Document's metadata and content
 * @param provider — Google Drive provider
 * @param documentId
 * @returns document
 * @returns content — Raw document content JSON
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataGoogleDocsGet({ provider: Struct, documentId: string }): { document: Struct, content: any, errorMessage: string };

/**
 * Extract plain text from a Google Document
 * @param provider — Google Drive provider
 * @param documentId
 * @returns text — Plain text content
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataGoogleDocsGetText({ provider: Struct, documentId: string }): { text: string, errorMessage: string };

/**
 * Insert text at a specific location in a Google Document
 * @param provider — Google Drive provider
 * @param documentId
 * @param text — Text to insert
 * @param index (optional) — Character index to insert at (1 = start)
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataGoogleDocsInsertText({ provider: Struct, documentId: string, text: string, index?: int }): string;

/**
 * Replace all occurrences of text in a Google Document
 * @param provider — Google Drive provider
 * @param documentId
 * @param searchText — Text to find
 * @param replaceText — Replacement text
 * @param matchCase (optional) — Case-sensitive search
 * @returns occurrences — Number of replacements
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataGoogleDocsReplaceText({ provider: Struct, documentId: string, searchText: string, replaceText: string, matchCase?: bool }): { occurrences: int, errorMessage: string };


// === Data/Google/Drive ===

/**
 * Copy a file in Google Drive
 * @param provider — Google Drive provider
 * @param fileId — ID of file to copy
 * @param newName (optional) — Name for the copy (empty to keep original)
 * @param parentId (optional) — Destination folder ID (empty for same location)
 * @returns newFileId — ID of the copied file
 * @returns file — Copied file details
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataGoogleDriveCopyFile({ provider: Struct, fileId: string, newName?: string, parentId?: string }): { newFileId: string, file: Struct, errorMessage: string };

/**
 * Create a new folder in Google Drive
 * @param provider — Google Drive provider
 * @param name — Folder name
 * @param parentId (optional) — Parent folder ID (empty for root)
 * @returns folderId — Created folder ID
 * @returns folder — Created folder details
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataGoogleDriveCreateFolder({ provider: Struct, name: string, parentId?: string }): { folderId: string, folder: Struct, errorMessage: string };

/**
 * Delete a file or folder from Google Drive
 * @param provider — Google Drive provider
 * @param fileId — ID of file/folder to delete
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataGoogleDriveDeleteFile({ provider: Struct, fileId: string }): string;

/**
 * Download a Google Drive file into a FlowPath
 * @param provider — Google Drive provider
 * @param fileId — File ID to download
 * @param exportMimeType (optional) — For Google Docs, export format (e.g., 'application/pdf')
 * @param outputPath — FlowPath to write the downloaded file into
 * @returns path — Written file path
 * @returns fileName — Drive file name
 * @returns mimeType — Downloaded or exported MIME type
 * @returns size — Downloaded size in bytes
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataGoogleDriveDownload({ provider: Struct, fileId: string, exportMimeType?: string, outputPath: Struct }): { path: Struct, fileName: string, mimeType: string, size: int, errorMessage: string };

/**
 * Get detailed metadata for a Google Drive file
 * @param provider — Google Drive provider
 * @param fileId — File ID
 * @returns file — File metadata
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataGoogleDriveGetMetadata({ provider: Struct, fileId: string }): { file: Struct, errorMessage: string };

/**
 * Lists files from a Google Drive folder
 * @param provider — Google Drive provider (from Google Drive node)
 * @param folderId (optional) — The ID of the folder to list files from. Use 'root' for the root folder.
 * @param query (optional) — Optional search query to filter files (e.g., 'name contains "report"')
 * @param pageSize (optional) — Maximum number of files to return (1-1000)
 * @param includeFolders (optional) — Whether to include folders in the results
 * @returns files — Array of Google Drive files
 * @returns fileCount — Number of files returned
 * @impure has side effects / drives control flow
 */
declare function dataGoogleDriveListFiles({ provider: Struct, folderId?: string, query?: string, pageSize?: int, includeFolders?: bool }): { files: Struct[], fileCount: int };

/**
 * Move a file to a different folder in Google Drive
 * @param provider — Google Drive provider
 * @param fileId — ID of file to move
 * @param newParentId — Destination folder ID
 * @returns file — Updated file details
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataGoogleDriveMoveFile({ provider: Struct, fileId: string, newParentId: string }): { file: Struct, errorMessage: string };

/**
 * Reads the content of a file from Google Drive as text
 * @param provider — Google Drive provider (from Google Drive node)
 * @param fileId — The ID of the file to read (from Google Drive)
 * @param exportMimeType (optional) — For Google Docs files, specify the export format (e.g., 'text/plain', 'application/pdf'). Leave empty for regular files.
 * @returns content — The text content of the file
 * @returns fileName — The name of the file
 * @returns mimeType — The MIME type of the file
 * @returns size — The size of the file in bytes
 * @impure has side effects / drives control flow
 */
declare function dataGoogleDriveReadFile({ provider: Struct, fileId: string, exportMimeType?: string }): { content: string, fileName: string, mimeType: string, size: int };

/**
 * Search for files in Google Drive
 * @param provider — Google Drive provider
 * @param query — Search query (supports Drive query syntax)
 * @param pageSize (optional) — Max results (1-1000)
 * @returns files — Search results
 * @returns count — Number of results
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataGoogleDriveSearch({ provider: Struct, query: string, pageSize?: int }): { files: Struct[], count: int, errorMessage: string };

/**
 * Upload a FlowPath file to Google Drive
 * @param provider — Google Drive provider
 * @param sourceFile — FlowPath file to upload
 * @param fileName (optional) — Destination filename. Leave empty to use the FlowPath filename.
 * @param parentId (optional) — Destination folder ID. Leave empty for My Drive root.
 * @param mimeType (optional) — Uploaded file MIME type
 * @returns fileId — Uploaded file ID
 * @returns file — Uploaded file metadata
 * @returns usedResumableUpload — True when Google Drive resumable upload was used
 * @returns size — Uploaded size in bytes
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataGoogleDriveUpload({ provider: Struct, sourceFile: Struct, fileName?: string, parentId?: string, mimeType?: string }): { fileId: string, file: Struct, usedResumableUpload: bool, size: int, errorMessage: string };


// === Data/Google/Forms ===

/**
 * Create a new Google Form
 * @param provider — Google provider
 * @param title — Form title
 * @param documentTitle (optional) — Document title (filename)
 * @returns form — Created form
 * @returns formId
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataGoogleFormsCreate({ provider: Struct, title: string, documentTitle?: string }): { form: Struct, formId: string, errorMessage: string };

/**
 * Get details of a Google Form
 * @param provider — Google provider
 * @param formId — The ID of the form
 * @returns form — Form details
 * @returns questions — Form questions
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataGoogleFormsGet({ provider: Struct, formId: string }): { form: Struct, questions: Struct[], errorMessage: string };

/**
 * Get a specific response from a Google Form
 * @param provider — Google provider
 * @param formId — The ID of the form
 * @param responseId — The ID of the response
 * @returns response — Form response
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataGoogleFormsGetResponse({ provider: Struct, formId: string, responseId: string }): { response: Struct, errorMessage: string };

/**
 * List all responses to a Google Form
 * @param provider — Google provider
 * @param formId — The ID of the form
 * @param pageToken (optional) — Token for pagination
 * @returns responses — Form responses
 * @returns nextPageToken
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataGoogleFormsListResponses({ provider: Struct, formId: string, pageToken?: string }): { responses: Struct[], nextPageToken: string, errorMessage: string };

/**
 * Update title and description of a Google Form
 * @param provider — Google provider
 * @param formId — The ID of the form
 * @param title (optional) — New form title
 * @param description (optional) — New form description
 * @returns form — Updated form
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataGoogleFormsUpdateInfo({ provider: Struct, formId: string, title?: string, description?: string }): { form: Struct, errorMessage: string };


// === Data/Google/Gmail ===

/**
 * Create a draft email in Gmail
 * @param provider — Google provider
 * @param to — Recipient email address(es)
 * @param subject — Email subject
 * @param body — Email body (plain text)
 * @param cc (optional) — CC recipients (optional)
 * @param bcc (optional) — BCC recipients (optional)
 * @returns draft — Created draft
 * @returns draftId — ID of the created draft
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataGoogleGmailCreateDraft({ provider: Struct, to: string, subject: string, body: string, cc?: string, bcc?: string }): { draft: Struct, draftId: string, errorMessage: string };

/**
 * List all labels in Gmail
 * @param provider — Google provider
 * @returns labels — List of labels
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataGoogleGmailListLabels({ provider: Struct }): { labels: Struct[], errorMessage: string };

/**
 * Send an email via Gmail
 * @param provider — Google provider
 * @param to — Recipient email address(es), comma-separated
 * @param subject — Email subject
 * @param body — Email body (plain text)
 * @param cc (optional) — CC recipients (optional)
 * @param bcc (optional) — BCC recipients (optional)
 * @returns messageId — ID of the sent message
 * @returns threadId — Thread ID of the message
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataGoogleGmailSend({ provider: Struct, to: string, subject: string, body: string, cc?: string, bcc?: string }): { messageId: string, threadId: string, errorMessage: string };


// === Data/Google/Meet ===

/**
 * Add Google Meet to an existing calendar event
 * @param provider — Google provider
 * @param eventId — Calendar event ID
 * @param calendarId (optional) — Calendar ID
 * @returns meetLink — Google Meet URL
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataGoogleMeetAddToEvent({ provider: Struct, eventId: string, calendarId?: string }): { meetLink: string, errorMessage: string };

/**
 * Create a new Google Meet meeting
 * @param provider — Google provider
 * @param summary — Meeting title
 * @param description (optional) — Meeting description
 * @param startTime — Start time (RFC3339, e.g., 2024-01-01T10:00:00-05:00)
 * @param durationMinutes (optional) — Meeting duration in minutes
 * @param timeZone (optional) — Time zone (e.g., America/New_York)
 * @param attendees (optional) — Comma-separated email addresses
 * @param sendInvites (optional) — Send calendar invitations
 * @returns meetInfo — Meeting information
 * @returns meetLink — Google Meet URL
 * @returns eventId — Calendar event ID
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataGoogleMeetCreate({ provider: Struct, summary: string, description?: string, startTime: string, durationMinutes?: int, timeZone?: string, attendees?: string, sendInvites?: bool }): { meetInfo: Struct, meetLink: string, eventId: string, errorMessage: string };

/**
 * Get details of a Google Meet meeting from its calendar event
 * @param provider — Google provider
 * @param eventId — Calendar event ID
 * @param calendarId (optional) — Calendar ID
 * @returns meetInfo — Meeting information
 * @returns meetLink — Google Meet URL
 * @returns hasMeet — Whether this event has a Google Meet
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataGoogleMeetGet({ provider: Struct, eventId: string, calendarId?: string }): { meetInfo: Struct, meetLink: string, hasMeet: bool, errorMessage: string };

/**
 * Create an instant Google Meet meeting starting now
 * @param provider — Google provider
 * @param summary (optional) — Meeting title
 * @param durationMinutes (optional) — Meeting duration
 * @returns meetLink — Google Meet URL
 * @returns eventId — Calendar event ID
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataGoogleMeetInstant({ provider: Struct, summary?: string, durationMinutes?: int }): { meetLink: string, eventId: string, errorMessage: string };


// === Data/Google/Sheets ===

/**
 * Add a new sheet to a Google Spreadsheet
 * @param provider — Google Drive provider
 * @param spreadsheetId
 * @param title — New sheet title
 * @returns sheetId — ID of the new sheet
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataGoogleSheetsAddSheet({ provider: Struct, spreadsheetId: string, title: string }): { sheetId: int, errorMessage: string };

/**
 * Append rows to the end of a Google Sheets range
 * @param provider — Google Drive provider
 * @param spreadsheetId
 * @param range — A1 notation range (e.g., 'Sheet1!A:D')
 * @param values — 2D array of row values to append
 * @returns updatedRange — Range that was updated
 * @returns updatedRows — Number of rows appended
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataGoogleSheetsAppendRows({ provider: Struct, spreadsheetId: string, range: string, values: any[] }): { updatedRange: string, updatedRows: int, errorMessage: string };

/**
 * Clear values from a Google Sheets range
 * @param provider — Google Drive provider
 * @param spreadsheetId
 * @param range — A1 notation range to clear
 * @returns clearedRange
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataGoogleSheetsClearRange({ provider: Struct, spreadsheetId: string, range: string }): { clearedRange: string, errorMessage: string };

/**
 * Create a new Google Spreadsheet
 * @param provider — Google Drive provider
 * @param title — Spreadsheet title
 * @returns spreadsheetId
 * @returns spreadsheet
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataGoogleSheetsCreate({ provider: Struct, title: string }): { spreadsheetId: string, spreadsheet: Struct, errorMessage: string };

/**
 * Delete a sheet from a Google Spreadsheet
 * @param provider — Google Drive provider
 * @param spreadsheetId
 * @param sheetId — ID of sheet to delete
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataGoogleSheetsDeleteSheet({ provider: Struct, spreadsheetId: string, sheetId: int }): string;

/**
 * Get Google Spreadsheet metadata and sheet list
 * @param provider — Google Drive provider
 * @param spreadsheetId — ID of the spreadsheet
 * @returns spreadsheet
 * @returns sheets — List of sheets
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataGoogleSheetsGet({ provider: Struct, spreadsheetId: string }): { spreadsheet: Struct, sheets: Struct[], errorMessage: string };

/**
 * Read data from a Google Sheets range
 * @param provider — Google Drive provider
 * @param spreadsheetId
 * @param range — A1 notation range (e.g., 'Sheet1!A1:D10')
 * @param valueRender (optional) — How values should be rendered
 * @returns values — 2D array of cell values
 * @returns rowCount
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataGoogleSheetsReadRange({ provider: Struct, spreadsheetId: string, range: string, valueRender?: string }): { values: any[], rowCount: int, errorMessage: string };

/**
 * Write data to a Google Sheets range
 * @param provider — Google Drive provider
 * @param spreadsheetId
 * @param range — A1 notation range
 * @param values — 2D array of values to write
 * @param valueInput (optional) — How input should be interpreted
 * @returns updatedCells — Number of cells updated
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataGoogleSheetsWriteRange({ provider: Struct, spreadsheetId: string, range: string, values: any[], valueInput?: string }): { updatedCells: int, errorMessage: string };


// === Data/Google/Slides ===

/**
 * Add a new slide to a Google Slides presentation
 * @param provider — Google Drive provider
 * @param presentationId
 * @param layout (optional) — Predefined layout for the slide
 * @param insertIndex (optional) — Index where to insert slide (optional)
 * @returns slideId — ID of the created slide
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataGoogleSlidesAddSlide({ provider: Struct, presentationId: string, layout?: string, insertIndex?: int }): { slideId: string, errorMessage: string };

/**
 * Add a text box with text to a Google Slide
 * @param provider — Google Drive provider
 * @param presentationId
 * @param slideId — ID of the slide
 * @param text — Text content to add
 * @param x (optional) — X position in EMU (914400 EMU = 1 inch)
 * @param y (optional) — Y position in EMU
 * @param width (optional) — Width in EMU
 * @param height (optional) — Height in EMU
 * @returns shapeId — ID of the created text box
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataGoogleSlidesAddText({ provider: Struct, presentationId: string, slideId: string, text: string, x?: float, y?: float, width?: float, height?: float }): { shapeId: string, errorMessage: string };

/**
 * Create a new Google Slides presentation
 * @param provider — Google Drive provider
 * @param title — Presentation title
 * @returns presentationId
 * @returns presentation
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataGoogleSlidesCreate({ provider: Struct, title: string }): { presentationId: string, presentation: Struct, errorMessage: string };

/**
 * Delete a slide from a Google Slides presentation
 * @param provider — Google Drive provider
 * @param presentationId
 * @param slideId — ID of the slide to delete
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataGoogleSlidesDeleteSlide({ provider: Struct, presentationId: string, slideId: string }): string;

/**
 * Export a Google Slides presentation into a FlowPath
 * @param provider — Google Drive provider
 * @param presentationId
 * @param format (optional) — Export format
 * @param outputPath — FlowPath to write the exported presentation into
 * @returns path — Written file path
 * @returns size — Exported size in bytes
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataGoogleSlidesExport({ provider: Struct, presentationId: string, format?: string, outputPath: Struct }): { path: Struct, size: int, errorMessage: string };

/**
 * Get a Google Slides presentation's metadata and slides
 * @param provider — Google Drive provider
 * @param presentationId
 * @returns presentation
 * @returns slides — List of slides
 * @returns slideCount
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataGoogleSlidesGet({ provider: Struct, presentationId: string }): { presentation: Struct, slides: Struct[], slideCount: int, errorMessage: string };


// === Data/Google/YouTube ===

/**
 * Add a video to a YouTube playlist
 * @param provider — Google provider
 * @param playlistId — YouTube playlist ID
 * @param videoId — YouTube video ID to add
 * @param position (optional) — Position in playlist (optional, -1 for end)
 * @returns itemId — ID of the playlist item
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataGoogleYoutubeAddToPlaylist({ provider: Struct, playlistId: string, videoId: string, position?: int }): { itemId: string, errorMessage: string };

/**
 * Get YouTube channel details
 * @param provider — Google provider
 * @param channelId (optional) — YouTube channel ID (leave empty for own channel)
 * @returns channel — Channel details
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataGoogleYoutubeGetChannel({ provider: Struct, channelId?: string }): { channel: Struct, errorMessage: string };

/**
 * Get videos in a YouTube playlist
 * @param provider — Google provider
 * @param playlistId — YouTube playlist ID
 * @param maxResults (optional) — Maximum results (1-50)
 * @param pageToken (optional) — Token for pagination
 * @returns items — Playlist items
 * @returns nextPageToken
 * @returns totalItems — Total items in playlist
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataGoogleYoutubeGetPlaylistItems({ provider: Struct, playlistId: string, maxResults?: int, pageToken?: string }): { items: Struct[], nextPageToken: string, totalItems: int, errorMessage: string };

/**
 * Get YouTube video details by ID
 * @param provider — Google provider
 * @param videoId — YouTube video ID
 * @returns video — Video details
 * @returns raw — Raw API response
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataGoogleYoutubeGetVideo({ provider: Struct, videoId: string }): { video: Struct, raw: any, errorMessage: string };

/**
 * List videos from the authenticated user's channel
 * @param provider — Google provider
 * @param maxResults (optional) — Maximum results (1-50)
 * @param pageToken (optional) — Token for pagination
 * @returns videos — List of videos
 * @returns nextPageToken
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataGoogleYoutubeListMyVideos({ provider: Struct, maxResults?: int, pageToken?: string }): { videos: Struct[], nextPageToken: string, errorMessage: string };

/**
 * List YouTube playlists
 * @param provider — Google provider
 * @param channelId (optional) — Channel ID (empty for own playlists)
 * @param maxResults (optional) — Maximum results (1-50)
 * @param pageToken (optional) — Token for pagination
 * @returns playlists — List of playlists
 * @returns nextPageToken
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataGoogleYoutubeListPlaylists({ provider: Struct, channelId?: string, maxResults?: int, pageToken?: string }): { playlists: Struct[], nextPageToken: string, errorMessage: string };

/**
 * Remove a video from a YouTube playlist
 * @param provider — Google provider
 * @param itemId — Playlist item ID (not video ID)
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataGoogleYoutubeRemoveFromPlaylist({ provider: Struct, itemId: string }): string;

/**
 * Search for YouTube videos
 * @param provider — Google provider
 * @param query — Search query
 * @param maxResults (optional) — Maximum number of results (1-50)
 * @param order (optional) — Sort order
 * @param pageToken (optional) — Token for pagination
 * @returns videos — List of videos
 * @returns nextPageToken
 * @returns totalResults — Estimated total results
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataGoogleYoutubeSearch({ provider: Struct, query: string, maxResults?: int, order?: string, pageToken?: string }): { videos: Struct[], nextPageToken: string, totalResults: int, errorMessage: string };


// === Data/LinkedIn ===

/**
 * Get the current authenticated user's LinkedIn profile information
 * @param provider — LinkedIn provider
 * @returns me — Current user's LinkedIn profile
 * @returns sub — The user's unique LinkedIn ID (sub claim)
 * @returns email — The user's email address
 * @returns name — The user's display name
 * @impure has side effects / drives control flow
 */
declare function dataLinkedinGetMe({ provider: Struct }): { me: Struct, sub: string, email: string, name: string };

/**
 * Connect to LinkedIn using OAuth 2.0. Requires OAuth provider configuration in flow-like.config.json.
 * @returns provider — LinkedIn provider for API access
 */
declare function dataLinkedinProviderOauth(): Struct;

/**
 * Share an article/link on LinkedIn with optional title and description
 * @param provider — LinkedIn provider
 * @param authorId — LinkedIn user ID (sub from Get Me node). Format: urn:li:person:{sub}
 * @param text — Commentary text for your article share
 * @param url — The URL of the article to share
 * @param title (optional) — Optional title for the article
 * @param description (optional) — Optional description for the article
 * @param visibility (optional) — Who can see this post: PUBLIC, CONNECTIONS
 * @returns postId — The ID of the created post
 * @returns errorMessage — Error message if article sharing fails
 * @impure has side effects / drives control flow
 */
declare function dataLinkedinShareArticle({ provider: Struct, authorId: string, text: string, url: string, title?: string, description?: string, visibility?: string }): { postId: string, errorMessage: string };

/**
 * Share a text post on LinkedIn
 * @param provider — LinkedIn provider
 * @param authorId — LinkedIn user ID (sub from Get Me node). Format: urn:li:person:{sub}
 * @param text — The text content of your post
 * @param visibility (optional) — Who can see this post: PUBLIC, CONNECTIONS
 * @returns postId — The ID of the created post
 * @returns errorMessage — Error message if post sharing fails
 * @impure has side effects / drives control flow
 */
declare function dataLinkedinShareText({ provider: Struct, authorId: string, text: string, visibility?: string }): { postId: string, errorMessage: string };


// === Data/Microsoft ===

/**
 * Call any Microsoft Graph endpoint with optional collection pagination
 * @param provider — Microsoft Graph provider
 * @param method (optional) — HTTP method
 * @param path — Graph path like /me/messages or an absolute Graph URL
 * @param body (optional) — JSON request body for POST, PATCH, or PUT
 * @param paginate (optional) — Follow @odata.nextLink for GET collection responses
 * @returns status — HTTP status code
 * @returns response — Raw JSON response
 * @returns values — Paginated collection values
 * @returns nextLink — @odata.nextLink
 * @returns deltaLink — @odata.deltaLink
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftGraphRequest({ provider: Struct, method?: string, path: string, body?: Struct, paginate?: bool }): { status: int, response: Struct, values: Struct[], nextLink: string, deltaLink: string, errorMessage: string };

/**
 * Connect to Microsoft Graph using OAuth Authorization Code Flow with PKCE.
 * @param baseUrl (optional) — Microsoft Graph API base URL
 * @returns provider — Microsoft Graph provider with authentication
 */
declare function dataMicrosoftProviderOauth({ baseUrl?: string }): Struct;

/**
 * Connect to Microsoft Graph API using an access token. Use for server-to-server auth or manual token management.
 * @param token — Microsoft Graph API access token
 * @param baseUrl (optional) — Microsoft Graph API base URL
 * @returns provider — Microsoft Graph provider with authentication
 */
declare function dataMicrosoftProviderToken({ token: string, baseUrl?: string }): Struct;


// === Data/Microsoft/Calendar ===

/**
 * Create a new calendar
 * @param provider — Microsoft Graph provider
 * @param name — Calendar name
 * @param color (optional) — Calendar color
 * @returns calendar
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftCalendarCreateCalendar({ provider: Struct, name: string, color?: string }): { calendar: Struct, errorMessage: string };

/**
 * Create a new calendar event
 * @param provider — Microsoft Graph provider
 * @param subject — Event subject
 * @param body (optional) — Event description (HTML)
 * @param startDateTime — Start date/time
 * @param endDateTime — End date/time
 * @param timeZone (optional) — Time zone
 * @param location (optional) — Event location
 * @param attendees (optional) — Comma-separated email addresses
 * @param isOnlineMeeting (optional) — Create Teams meeting
 * @returns event
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftCalendarCreateEvent({ provider: Struct, subject: string, body?: string, startDateTime: Date, endDateTime: Date, timeZone?: string, location?: string, attendees?: string, isOnlineMeeting?: bool }): { event: Struct, errorMessage: string };

/**
 * Delete a calendar event
 * @param provider — Microsoft Graph provider
 * @param eventId — ID of the event
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftCalendarDeleteEvent({ provider: Struct, eventId: string }): string;

/**
 * Find available meeting times for attendees
 * @param provider — Microsoft Graph provider
 * @param attendees — Comma-separated email addresses
 * @param durationMinutes (optional) — Meeting duration in minutes
 * @param startDate — Start of search window
 * @param endDate — End of search window
 * @param timeZone (optional) — Time zone
 * @returns suggestions
 * @returns count
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftCalendarFindMeetingTimes({ provider: Struct, attendees: string, durationMinutes?: int, startDate: Date, endDate: Date, timeZone?: string }): { suggestions: Struct[], count: int, errorMessage: string };

/**
 * Get free/busy schedule for users
 * @param provider — Microsoft Graph provider
 * @param schedules — Comma-separated email addresses
 * @param startDateTime — Start date/time
 * @param endDateTime — End date/time
 * @param timeZone (optional) — Time zone
 * @param intervalMinutes (optional) — Availability interval in minutes
 * @returns scheduleData — Raw schedule response
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftCalendarGetSchedule({ provider: Struct, schedules: string, startDateTime: Date, endDateTime: Date, timeZone?: string, intervalMinutes?: int }): { scheduleData: Struct, errorMessage: string };

/**
 * List all calendars for the user
 * @param provider — Microsoft Graph provider
 * @returns calendars
 * @returns count
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftCalendarListCalendars({ provider: Struct }): { calendars: Struct[], count: int, errorMessage: string };

/**
 * List calendar events within a time range
 * @param provider — Microsoft Graph provider
 * @param calendarId (optional) — ID of the calendar (optional, uses default)
 * @param startDate — Start date
 * @param endDate — End date
 * @param top (optional) — Maximum number of events to return
 * @returns events
 * @returns count
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftCalendarListEvents({ provider: Struct, calendarId?: string, startDate: Date, endDate: Date, top?: int }): { events: Struct[], count: int, errorMessage: string };

/**
 * Update an existing calendar event
 * @param provider — Microsoft Graph provider
 * @param eventId — ID of the event
 * @param subject (optional) — New subject (leave empty to keep)
 * @param startDateTime — New start (leave empty to keep)
 * @param endDateTime — New end (leave empty to keep)
 * @param timeZone (optional) — Time zone for dates
 * @param location (optional) — New location (leave empty to keep)
 * @returns event
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftCalendarUpdateEvent({ provider: Struct, eventId: string, subject?: string, startDateTime: Date, endDateTime: Date, timeZone?: string, location?: string }): { event: Struct, errorMessage: string };


// === Data/Microsoft/Copilot ===

/**
 * Send a message to Microsoft 365 Copilot using the official Chat API with streaming support. Supports file context from OneDrive/SharePoint and web search grounding.
 * @param provider — Microsoft Graph provider
 * @param prompt — User message to send to Copilot
 * @param additionalContext (optional) — Extra grounding context (e.g., document excerpts, facts) to provide to Copilot
 * @param fileUrls (optional) — OneDrive/SharePoint file URLs to include as context (full URLs like https://contoso.sharepoint.com/...)
 * @param webGrounding (optional) — Enable web search grounding for real-time information
 * @param timezone (optional) — User timezone in IANA format (e.g., America/New_York, Europe/London). Auto-detected from system if empty.
 * @param conversationId (optional) — Optional conversation ID to continue a chat (leave empty for new conversation)
 * @returns chunk — Streaming chunk
 * @returns result — Complete response with annotations from citations
 * @returns response — Full Copilot response with attributions and adaptive cards
 * @returns attachments — Attachments created from Copilot's attributions (citations and references)
 * @returns newConversationId — Conversation ID for follow-up messages
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftCopilotChat({ provider: Struct, prompt: string, additionalContext?: string[], fileUrls?: string[], webGrounding?: bool, timezone?: string, conversationId?: string }): { chunk: Struct, result: Struct, response: Struct, attachments: Struct[], newConversationId: string, errorMessage: string };

/**
 * Filter Copilot interactions by type (user prompts vs AI responses)
 * @param interactions — Copilot interactions to filter
 * @param interactionType (optional) — Type to filter by
 * @returns filtered
 * @returns count
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftCopilotFilterInteractions({ interactions: Struct[], interactionType?: string }): { filtered: Struct[], count: int };

/**
 * Get Microsoft 365 Copilot interaction history (prompts and responses)
 * @param provider — Microsoft Graph provider
 * @param userId — User ID to get interactions for
 * @param appClassFilter (optional) — Filter by app (e.g., IPM.SkypeTeams.Message.Copilot.BizChat)
 * @param top (optional) — Maximum number of results
 * @param useBeta (optional) — Use beta endpoint instead of v1.0
 * @returns interactions
 * @returns count
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftCopilotGetInteractions({ provider: Struct, userId: string, appClassFilter?: string, top?: int, useBeta?: bool }): { interactions: Struct[], count: int, errorMessage: string };

/**
 * Get a specific AI insight from a Teams meeting
 * @param provider — Microsoft Graph provider
 * @param meetingId — Online meeting ID
 * @param insightId — AI insight ID
 * @returns insight
 * @returns actionItems
 * @returns meetingNotes
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftCopilotGetMeetingInsight({ provider: Struct, meetingId: string, insightId: string }): { insight: Struct, actionItems: Struct[], meetingNotes: Struct[], errorMessage: string };

/**
 * Get the current user's Copilot settings and preferences
 * @param provider — Microsoft Graph provider
 * @returns settings — Raw settings data
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftCopilotGetUserSettings({ provider: Struct }): { settings: Struct, errorMessage: string };

/**
 * Get AI-generated meeting notes and action items from Teams meetings
 * @param provider — Microsoft Graph provider
 * @param meetingId — Online meeting ID
 * @returns insights
 * @returns count
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftCopilotListMeetingInsights({ provider: Struct, meetingId: string }): { insights: Struct[], count: int, errorMessage: string };

/**
 * Perform hybrid semantic and lexical search across OneDrive for work or school content using the official Microsoft 365 Copilot Search API
 * @param provider — Microsoft Graph provider
 * @param query — Natural language query (max 1500 characters)
 * @param pageSize (optional) — Number of results per page (1-100, default: 25)
 * @param filterExpression (optional) — Optional KQL path filter (e.g., path:"https://contoso.sharepoint.com/...")
 * @param resourceMetadata (optional) — Optional comma-separated metadata fields to return (e.g., title,author)
 * @returns results — Search results
 * @returns totalCount — Total number of results available
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftCopilotSemanticSearch({ provider: Struct, query: string, pageSize?: int, filterExpression?: string, resourceMetadata?: string }): { results: Struct[], totalCount: int, errorMessage: string };

/**
 * Subscribe to change notifications for Copilot interactions
 * @param provider — Microsoft Graph provider
 * @param notificationUrl — Webhook URL to receive notifications
 * @param expirationMinutes (optional) — Subscription expiration time
 * @param clientState (optional) — Optional client state for validation
 * @returns subscriptionId
 * @returns expirationDateTime
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftCopilotSubscribeNotifications({ provider: Struct, notificationUrl: string, expirationMinutes?: int, clientState?: string }): { subscriptionId: string, expirationDateTime: string, errorMessage: string };


// === Data/Microsoft/Excel ===

/**
 * Add a row to an Excel table
 * @param provider — Microsoft Graph provider
 * @param filePath — Path to Excel file
 * @param tableName — Name of the table
 * @param values — Array of values for the row
 * @returns rowIndex — Index of added row
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftExcelAddTableRow({ provider: Struct, filePath: string, tableName: string, values: any[] }): { rowIndex: int, errorMessage: string };

/**
 * Get data from an Excel table by name
 * @param provider — Microsoft Graph provider
 * @param filePath — Path to Excel file
 * @param tableName — Name of the table
 * @returns rows — Table rows as array
 * @returns headers — Column headers
 * @returns rowCount
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftExcelGetTable({ provider: Struct, filePath: string, tableName: string }): { rows: any[], headers: string[], rowCount: int, errorMessage: string };

/**
 * List worksheets in an Excel workbook stored in OneDrive
 * @param provider — Microsoft Graph provider
 * @param filePath — Path to Excel file in OneDrive
 * @returns worksheets
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftExcelListWorksheets({ provider: Struct, filePath: string }): { worksheets: Struct[], errorMessage: string };

/**
 * Read data from a range in an Excel worksheet
 * @param provider — Microsoft Graph provider
 * @param filePath — Path to Excel file in OneDrive
 * @param worksheet — Worksheet name
 * @param range — A1 notation range
 * @returns values — 2D array of cell values
 * @returns rowCount
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftExcelReadRange({ provider: Struct, filePath: string, worksheet: string, range: string }): { values: any[], rowCount: int, errorMessage: string };

/**
 * Get the used range of a worksheet
 * @param provider — Microsoft Graph provider
 * @param filePath — Path to Excel file
 * @param worksheet — Worksheet name
 * @returns values — 2D array of data
 * @returns address — A1 notation address
 * @returns rowCount
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftExcelUsedRange({ provider: Struct, filePath: string, worksheet: string }): { values: any[], address: string, rowCount: int, errorMessage: string };

/**
 * Write data to a range in an Excel worksheet
 * @param provider — Microsoft Graph provider
 * @param filePath — Path to Excel file in OneDrive
 * @param worksheet — Worksheet name
 * @param range — A1 notation range
 * @param values — 2D array of values to write
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftExcelWriteRange({ provider: Struct, filePath: string, worksheet: string, range: string, values: any[] }): string;


// === Data/Microsoft/OneDrive ===

/**
 * Copy a file or folder in OneDrive
 * @param provider — Microsoft Graph provider
 * @param itemPath — Path to the item to copy
 * @param destinationPath — Path to destination folder
 * @param newName (optional) — Optional name for the copy
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftOnedriveCopy({ provider: Struct, itemPath: string, destinationPath: string, newName?: string }): string;

/**
 * Create a new folder in OneDrive
 * @param provider — Microsoft Graph provider
 * @param parentPath (optional) — Path to parent folder (empty for root)
 * @param folderName — Name of the new folder
 * @returns item — Created folder metadata
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftOnedriveCreateFolder({ provider: Struct, parentPath?: string, folderName: string }): { item: Struct, errorMessage: string };

/**
 * Delete a file or folder from OneDrive
 * @param provider — Microsoft Graph provider
 * @param itemPath — Path to the item to delete
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftOnedriveDelete({ provider: Struct, itemPath: string }): string;

/**
 * Download a file from OneDrive
 * @param provider — Microsoft Graph provider
 * @param itemPath — Path to the file
 * @returns content — File content (base64 for binary)
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftOnedriveDownload({ provider: Struct, itemPath: string }): { content: string, errorMessage: string };

/**
 * Get metadata for a OneDrive item
 * @param provider — Microsoft Graph provider
 * @param itemPath — Path to the item
 * @returns item — OneDrive item metadata
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftOnedriveGetItem({ provider: Struct, itemPath: string }): { item: Struct, errorMessage: string };

/**
 * List files and folders in OneDrive
 * @param provider — Microsoft Graph provider
 * @param folderPath (optional) — Path to folder (empty for root)
 * @returns items — List of OneDrive items
 * @returns count
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftOnedriveListItems({ provider: Struct, folderPath?: string }): { items: Struct[], count: int, errorMessage: string };

/**
 * Move a file or folder to a new location in OneDrive
 * @param provider — Microsoft Graph provider
 * @param itemPath — Path to the item to move
 * @param destinationPath — Path to destination folder
 * @param newName (optional) — Optional new name for the item
 * @returns item — Moved item metadata
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftOnedriveMove({ provider: Struct, itemPath: string, destinationPath: string, newName?: string }): { item: Struct, errorMessage: string };

/**
 * Search for files and folders in OneDrive
 * @param provider — Microsoft Graph provider
 * @param query — Search query
 * @returns items — Search results
 * @returns count
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftOnedriveSearch({ provider: Struct, query: string }): { items: Struct[], count: int, errorMessage: string };

/**
 * Upload a FlowPath file to OneDrive; automatically uses an upload session for larger files
 * @param provider — Microsoft Graph provider
 * @param filePath (optional) — Destination path including filename. Leave empty to use the FlowPath filename.
 * @param file — FlowPath file to upload
 * @param conflictBehavior (optional) — What to do on conflict
 * @returns item — Uploaded item metadata
 * @returns usedUploadSession — True when large-file upload session was used
 * @returns size — Uploaded size in bytes
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftOnedriveUpload({ provider: Struct, filePath?: string, file: Struct, conflictBehavior?: string }): { item: Struct, usedUploadSession: bool, size: int, errorMessage: string };


// === Data/Microsoft/OneNote ===

/**
 * Create a new OneNote notebook
 * @param provider — Microsoft Graph provider
 * @param displayName — Name of the notebook
 * @returns notebook
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftOnenoteCreateNotebook({ provider: Struct, displayName: string }): { notebook: Struct, errorMessage: string };

/**
 * Create a new page in a OneNote section
 * @param provider — Microsoft Graph provider
 * @param sectionId — ID of the section
 * @param title — Page title
 * @param content (optional) — HTML content for the page body
 * @returns page
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftOnenoteCreatePage({ provider: Struct, sectionId: string, title: string, content?: string }): { page: Struct, errorMessage: string };

/**
 * Create a new section in a OneNote notebook
 * @param provider — Microsoft Graph provider
 * @param notebookId — ID of the notebook
 * @param displayName — Name of the section
 * @returns section
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftOnenoteCreateSection({ provider: Struct, notebookId: string, displayName: string }): { section: Struct, errorMessage: string };

/**
 * Delete a OneNote page
 * @param provider — Microsoft Graph provider
 * @param pageId — ID of the page
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftOnenoteDeletePage({ provider: Struct, pageId: string }): string;

/**
 * Get the HTML content of a OneNote page
 * @param provider — Microsoft Graph provider
 * @param pageId — ID of the page
 * @returns content — HTML content of the page
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftOnenoteGetPageContent({ provider: Struct, pageId: string }): { content: string, errorMessage: string };

/**
 * List all OneNote notebooks
 * @param provider — Microsoft Graph provider
 * @returns notebooks
 * @returns count
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftOnenoteListNotebooks({ provider: Struct }): { notebooks: Struct[], count: int, errorMessage: string };

/**
 * List all pages in a OneNote section
 * @param provider — Microsoft Graph provider
 * @param sectionId — ID of the section
 * @returns pages
 * @returns count
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftOnenoteListPages({ provider: Struct, sectionId: string }): { pages: Struct[], count: int, errorMessage: string };

/**
 * List all sections in a OneNote notebook
 * @param provider — Microsoft Graph provider
 * @param notebookId — ID of the notebook
 * @returns sections
 * @returns count
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftOnenoteListSections({ provider: Struct, notebookId: string }): { sections: Struct[], count: int, errorMessage: string };


// === Data/Microsoft/Outlook ===

/**
 * Access Outlook attachment fields and bytes
 * @param attachment — Outlook attachment struct
 * @returns id — Attachment ID
 * @returns filename — Attachment filename
 * @returns contentType — Attachment MIME type
 * @returns size — Attachment size in bytes
 * @returns isInline — Whether the attachment is inline
 * @returns contentId — Inline content ID
 * @returns contentLocation — Inline content location
 * @returns attachmentType — Graph attachment type
 * @returns data — Raw attachment bytes
 */
declare function dataMicrosoftOutlookAttachmentFields({ attachment: Struct }): { id: string, filename: string, contentType: string, size: int, isInline: bool, contentId: string, contentLocation: string, attachmentType: string, data: bytes[] };

/**
 * Create a new Outlook calendar event
 * @param provider — Microsoft Graph provider
 * @param subject — Event subject
 * @param body (optional) — Event description (HTML)
 * @param startDateTime — Start date/time (ISO format)
 * @param endDateTime — End date/time (ISO format)
 * @param timeZone (optional) — Time zone (e.g., 'UTC', 'Pacific Standard Time')
 * @param location (optional) — Event location
 * @param attendees (optional) — Comma-separated email addresses
 * @param isAllDay (optional) — Whether this is an all-day event
 * @param isOnlineMeeting (optional) — Create as Teams meeting
 * @param importance (optional) — Event importance
 * @returns event — Created event
 * @returns eventId — Created event ID
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftOutlookCreateCalendarEvent({ provider: Struct, subject: string, body?: string, startDateTime: string, endDateTime: string, timeZone?: string, location?: string, attendees?: string, isAllDay?: bool, isOnlineMeeting?: bool, importance?: string }): { event: Struct, eventId: string, errorMessage: string };

/**
 * Delete an Outlook calendar event
 * @param provider — Microsoft Graph provider
 * @param eventId — ID of the event to delete
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftOutlookDeleteCalendarEvent({ provider: Struct, eventId: string }): string;

/**
 * Forward a calendar event to other recipients
 * @param provider — Microsoft Graph provider
 * @param eventId — ID of the event to forward
 * @param toRecipients — Comma-separated email addresses
 * @param comment (optional) — Optional comment to include
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftOutlookForwardCalendarEvent({ provider: Struct, eventId: string, toRecipients: string, comment?: string }): string;

/**
 * Get a single Outlook calendar event by ID
 * @param provider — Microsoft Graph provider
 * @param eventId — ID of the event
 * @returns event — The calendar event
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftOutlookGetCalendarEvent({ provider: Struct, eventId: string }): { event: Struct, errorMessage: string };

/**
 * Get a single Outlook email message by ID
 * @param provider — Microsoft Graph provider
 * @param messageId — The message ID
 * @returns message — The email message
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftOutlookGetMessage({ provider: Struct, messageId: string }): { message: Struct, errorMessage: string };

/**
 * Fetch attachments for an Outlook message
 * @param provider — Microsoft Graph provider
 * @param message — Outlook message from List Messages or Get Message
 * @returns attachments — List of message attachments
 * @returns count — Number of attachments
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftOutlookGetMessageAttachments({ provider: Struct, message: Struct }): { attachments: Struct[], count: int, errorMessage: string };

/**
 * List Outlook calendar events
 * @param provider — Microsoft Graph provider
 * @param startDateTime (optional) — Start of time range (ISO 8601 format)
 * @param endDateTime (optional) — End of time range (ISO 8601 format)
 * @param top (optional) — Maximum events to return
 * @returns events — List of calendar events
 * @returns count — Number of events
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftOutlookListCalendarEvents({ provider: Struct, startDateTime?: string, endDateTime?: string, top?: int }): { events: Struct[], count: int, errorMessage: string };

/**
 * List Outlook contacts
 * @param provider — Microsoft Graph provider
 * @param search (optional) — Search term to filter contacts
 * @param top (optional) — Maximum contacts to return
 * @returns contacts — List of contacts
 * @returns count — Number of contacts
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftOutlookListContacts({ provider: Struct, search?: string, top?: int }): { contacts: Struct[], count: int, errorMessage: string };

/**
 * List Outlook mail folders
 * @param provider — Microsoft Graph provider
 * @returns folders — List of mail folders
 * @returns count — Number of folders
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftOutlookListMailFolders({ provider: Struct }): { folders: Struct[], count: int, errorMessage: string };

/**
 * List Outlook email messages
 * @param provider — Microsoft Graph provider
 * @param folderId (optional) — Mail folder ID (empty for inbox)
 * @param filter (optional) — OData filter (e.g., 'isRead eq false')
 * @param top (optional) — Maximum messages to return
 * @returns messages — List of email messages
 * @returns count — Number of messages
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftOutlookListMessages({ provider: Struct, folderId?: string, filter?: string, top?: int }): { messages: Struct[], count: int, errorMessage: string };

/**
 * Accept, decline, or tentatively accept a calendar event invitation
 * @param provider — Microsoft Graph provider
 * @param eventId — ID of the event
 * @param response (optional) — Your response
 * @param comment (optional) — Optional comment to send with response
 * @param sendResponse (optional) — Whether to send a response to the organizer
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftOutlookRsvpCalendarEvent({ provider: Struct, eventId: string, response?: string, comment?: string, sendResponse?: bool }): string;

/**
 * Send an email through Outlook
 * @param provider — Microsoft Graph provider
 * @param to — Recipient email addresses (comma-separated)
 * @param cc (optional) — CC recipients (comma-separated)
 * @param subject — Email subject
 * @param body — Email body content
 * @param isHtml (optional) — Whether the body is HTML content
 * @param attachments — Optional file attachments to include in the message
 * @param saveToSentItems (optional) — Save the message to Sent Items folder
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftOutlookSendMessage({ provider: Struct, to: string, cc?: string, subject: string, body: string, isHtml?: bool, attachments: Struct[], saveToSentItems?: bool }): string;

/**
 * Update an existing Outlook calendar event
 * @param provider — Microsoft Graph provider
 * @param eventId — ID of the event to update
 * @param subject (optional) — New subject (empty to keep)
 * @param body (optional) — New body (empty to keep)
 * @param startDateTime (optional) — New start (empty to keep)
 * @param endDateTime (optional) — New end (empty to keep)
 * @param timeZone (optional) — Time zone for dates
 * @param location (optional) — New location (empty to keep)
 * @returns event — Updated event
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftOutlookUpdateCalendarEvent({ provider: Struct, eventId: string, subject?: string, body?: string, startDateTime?: string, endDateTime?: string, timeZone?: string, location?: string }): { event: Struct, errorMessage: string };


// === Data/Microsoft/Planner ===

/**
 * Create a new bucket in a Planner plan
 * @param provider — Microsoft Graph provider
 * @param planId — ID of the plan
 * @param name — Bucket name
 * @returns bucket
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftPlannerCreateBucket({ provider: Struct, planId: string, name: string }): { bucket: Struct, errorMessage: string };

/**
 * Create a new task in a Planner plan
 * @param provider — Microsoft Graph provider
 * @param planId — ID of the plan
 * @param title — Task title
 * @param bucketId (optional) — ID of the bucket (optional)
 * @param dueDate (optional) — Due date (ISO format)
 * @param priority (optional) — Task priority (1=urgent, 3=important, 5=medium, 9=low)
 * @returns task
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftPlannerCreateTask({ provider: Struct, planId: string, title: string, bucketId?: string, dueDate?: string, priority?: int }): { task: Struct, errorMessage: string };

/**
 * Get details of a specific Planner plan
 * @param provider — Microsoft Graph provider
 * @param planId — ID of the plan
 * @returns plan
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftPlannerGetPlan({ provider: Struct, planId: string }): { plan: Struct, errorMessage: string };

/**
 * List all buckets in a Planner plan
 * @param provider — Microsoft Graph provider
 * @param planId — ID of the plan
 * @returns buckets
 * @returns count
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftPlannerListBuckets({ provider: Struct, planId: string }): { buckets: Struct[], count: int, errorMessage: string };

/**
 * List all Planner plans the user has access to
 * @param provider — Microsoft Graph provider
 * @returns plans
 * @returns count
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftPlannerListMyPlans({ provider: Struct }): { plans: Struct[], count: int, errorMessage: string };

/**
 * List all Planner tasks assigned to the current user
 * @param provider — Microsoft Graph provider
 * @returns tasks
 * @returns count
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftPlannerListMyTasks({ provider: Struct }): { tasks: Struct[], count: int, errorMessage: string };

/**
 * List all tasks in a Planner plan
 * @param provider — Microsoft Graph provider
 * @param planId — ID of the plan
 * @returns tasks
 * @returns count
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftPlannerListTasks({ provider: Struct, planId: string }): { tasks: Struct[], count: int, errorMessage: string };

/**
 * Update an existing Planner task
 * @param provider — Microsoft Graph provider
 * @param taskId — ID of the task
 * @param etag — Current ETag of the task
 * @param title (optional) — New task title (leave empty to keep)
 * @param percentComplete (optional) — Completion percentage (0-100)
 * @param priority (optional) — Task priority (-1 to keep)
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftPlannerUpdateTask({ provider: Struct, taskId: string, etag: string, title?: string, percentComplete?: int, priority?: int }): string;


// === Data/Microsoft/Search ===

/**
 * Search across Microsoft 365 content using the Microsoft Graph Search API. Supports files, emails, calendar events, Teams messages, SharePoint sites, and more.
 * @param provider — Microsoft Graph provider
 * @param query — Search query (supports KQL syntax for advanced queries)
 * @param entityTypes (optional) — Comma-separated entity types to search
 * @param size (optional) — Maximum number of results per page (default: 25, max: 1000 for SharePoint/OneDrive, 25 for message/event)
 * @param from (optional) — Starting offset for pagination (0-based)
 * @param fields (optional) — Comma-separated list of fields to return (empty for default)
 * @returns results — Search results (array of GraphSearchHit)
 * @returns count — Number of results returned
 * @returns total — Total estimated results available
 * @returns moreResults — Whether more results are available
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftGraphSearch({ provider: Struct, query: string, entityTypes?: string, size?: int, from?: int, fields?: string }): { results: Struct[], count: int, total: int, moreResults: bool, errorMessage: string };


// === Data/Microsoft/SharePoint ===

/**
 * Copy a SharePoint drive item asynchronously
 * @param provider — Microsoft Graph provider
 * @param driveId — Source drive ID
 * @param itemId — Drive item ID
 * @param destinationFolderId — Destination folder item ID
 * @param destinationDriveId (optional) — Optional destination drive ID. Leave empty to use Source Drive ID.
 * @param newName (optional) — Optional name for the copy
 * @param conflictBehavior (optional) — What to do on conflict
 * @param childrenOnly (optional) — Copy only children of a folder
 * @param includeAllVersionHistory (optional) — Copy all version history when supported
 * @returns monitorUrl — URL to monitor the asynchronous copy operation
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftSharepointCopyDriveItem({ provider: Struct, driveId: string, itemId: string, destinationFolderId: string, destinationDriveId?: string, newName?: string, conflictBehavior?: string, childrenOnly?: bool, includeAllVersionHistory?: bool }): { monitorUrl: string, errorMessage: string };

/**
 * Create a folder in a SharePoint drive
 * @param provider — Microsoft Graph provider
 * @param driveId — Drive ID
 * @param parentPath (optional) — Parent folder path when Parent ID is empty
 * @param parentId (optional) — Parent folder item ID. Takes precedence over Parent Path.
 * @param folderName — Name of the new folder
 * @param conflictBehavior (optional) — What to do on conflict
 * @returns item — Created folder metadata
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftSharepointCreateFolder({ provider: Struct, driveId: string, parentPath?: string, parentId?: string, folderName: string, conflictBehavior?: string }): { item: Struct, errorMessage: string };

/**
 * Create a SharePoint list item from field values
 * @param provider — Microsoft Graph provider
 * @param siteId — SharePoint site ID
 * @param listId — SharePoint list ID
 * @param fields — Field values keyed by internal field name
 * @returns item — Created list item
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftSharepointCreateListItem({ provider: Struct, siteId: string, listId: string, fields: Struct }): { item: Struct, errorMessage: string };

/**
 * Delete a file or folder from a SharePoint drive
 * @param provider — Microsoft Graph provider
 * @param driveId — Drive ID
 * @param itemId — Drive item ID
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftSharepointDeleteDriveItem({ provider: Struct, driveId: string, itemId: string }): string;

/**
 * Delete a SharePoint list item
 * @param provider — Microsoft Graph provider
 * @param siteId — SharePoint site ID
 * @param listId — SharePoint list ID
 * @param itemId — List item ID
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftSharepointDeleteListItem({ provider: Struct, siteId: string, listId: string, itemId: string }): string;

/**
 * Download a file from SharePoint
 * @param provider — Microsoft Graph provider
 * @param driveId — Drive ID
 * @param itemId — File item ID
 * @returns content — File content as bytes
 * @returns downloadUrl — Temporary download URL
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftSharepointDownloadFile({ provider: Struct, driveId: string, itemId: string }): { content: bytes, downloadUrl: string, errorMessage: string };

/**
 * Get metadata for a SharePoint drive item by ID or path
 * @param provider — Microsoft Graph provider
 * @param driveId — Drive ID
 * @param itemId (optional) — Drive item ID. Takes precedence over Item Path.
 * @param itemPath (optional) — Path to the drive item when Item ID is empty
 * @returns item — Drive item metadata
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftSharepointGetDriveItem({ provider: Struct, driveId: string, itemId?: string, itemPath?: string }): { item: Struct, errorMessage: string };

/**
 * Get a single SharePoint list item
 * @param provider — Microsoft Graph provider
 * @param siteId — SharePoint site ID
 * @param listId — SharePoint list ID
 * @param itemId — List item ID
 * @param expandFields (optional) — Include field values in response
 * @returns item — List item
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftSharepointGetListItem({ provider: Struct, siteId: string, listId: string, itemId: string, expandFields?: bool }): { item: Struct, errorMessage: string };

/**
 * Get items from a SharePoint list
 * @param provider — Microsoft Graph provider
 * @param siteId — SharePoint site ID
 * @param listId — SharePoint list ID
 * @param expandFields (optional) — Include field values in response
 * @returns items — List items
 * @returns count — Number of items
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftSharepointGetListItems({ provider: Struct, siteId: string, listId: string, expandFields?: bool }): { items: Struct[], count: int, errorMessage: string };

/**
 * Get a SharePoint site by hostname and path or site ID
 * @param provider — Microsoft Graph provider
 * @param hostname — SharePoint hostname (e.g., 'contoso.sharepoint.com')
 * @param sitePath (optional) — Site path (e.g., '/sites/marketing')
 * @param siteId (optional) — Alternatively, provide the site ID directly
 * @returns site — SharePoint site
 * @returns resolvedSiteId — The site ID
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftSharepointGetSite({ provider: Struct, hostname: string, sitePath?: string, siteId?: string }): { site: Struct, resolvedSiteId: string, errorMessage: string };

/**
 * List files and folders in a SharePoint drive (document library)
 * @param provider — Microsoft Graph provider
 * @param driveId — Drive (document library) ID
 * @param folderPath (optional) — Path to folder (empty for root, e.g., '/Documents/Reports')
 * @param folderId (optional) — Alternatively, provide folder item ID
 * @returns items — List of files and folders
 * @returns count — Number of items
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftSharepointListDriveItems({ provider: Struct, driveId: string, folderPath?: string, folderId?: string }): { items: Struct[], count: int, errorMessage: string };

/**
 * List document libraries (drives) in a SharePoint site
 * @param provider — Microsoft Graph provider
 * @param siteId — SharePoint site ID
 * @returns drives — List of document libraries
 * @returns count — Number of drives
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftSharepointListDrives({ provider: Struct, siteId: string }): { drives: Struct[], count: int, errorMessage: string };

/**
 * List all SharePoint lists in a site
 * @param provider — Microsoft Graph provider
 * @param siteId — SharePoint site ID
 * @returns lists — List of SharePoint lists
 * @returns count — Number of lists
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftSharepointListLists({ provider: Struct, siteId: string }): { lists: Struct[], count: int, errorMessage: string };

/**
 * Move or rename a SharePoint drive item
 * @param provider — Microsoft Graph provider
 * @param driveId — Source drive ID
 * @param itemId — Drive item ID
 * @param destinationFolderId — Destination folder item ID
 * @param destinationDriveId (optional) — Optional destination drive ID. Leave empty to use Source Drive ID.
 * @param newName (optional) — Optional new name for the item
 * @returns item — Moved drive item
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftSharepointMoveDriveItem({ provider: Struct, driveId: string, itemId: string, destinationFolderId: string, destinationDriveId?: string, newName?: string }): { item: Struct, errorMessage: string };

/**
 * Search files and folders in a SharePoint drive
 * @param provider — Microsoft Graph provider
 * @param driveId — Drive ID
 * @param query — Search query
 * @returns items — Search results
 * @returns count — Number of items
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftSharepointSearchDriveItems({ provider: Struct, driveId: string, query: string }): { items: Struct[], count: int, errorMessage: string };

/**
 * Search for SharePoint sites by keyword
 * @param provider — Microsoft Graph provider
 * @param query — Search term to find sites
 * @returns sites — List of matching SharePoint sites
 * @returns count — Number of sites found
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftSharepointSearchSites({ provider: Struct, query: string }): { sites: Struct[], count: int, errorMessage: string };

/**
 * Update field values on a SharePoint list item
 * @param provider — Microsoft Graph provider
 * @param siteId — SharePoint site ID
 * @param listId — SharePoint list ID
 * @param itemId — List item ID
 * @param fields — Field values keyed by internal field name
 * @returns updatedFields — Updated field value set
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftSharepointUpdateListItemFields({ provider: Struct, siteId: string, listId: string, itemId: string, fields: Struct }): { updatedFields: Struct, errorMessage: string };

/**
 * Upload a FlowPath file to a SharePoint drive; automatically uses an upload session for larger files
 * @param provider — Microsoft Graph provider
 * @param driveId — Drive ID
 * @param filePath (optional) — Destination path including filename. Leave empty to use the FlowPath filename.
 * @param file — FlowPath file to upload
 * @param conflictBehavior (optional) — What to do on conflict
 * @returns item — Uploaded drive item metadata
 * @returns usedUploadSession — True when large-file upload session was used
 * @returns size — Uploaded size in bytes
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftSharepointUploadFile({ provider: Struct, driveId: string, filePath?: string, file: Struct, conflictBehavior?: string }): { item: Struct, usedUploadSession: bool, size: int, errorMessage: string };


// === Data/Microsoft/Teams ===

/**
 * Create a new channel in a Microsoft Team
 * @param provider — Microsoft Graph provider
 * @param teamId — ID of the team
 * @param displayName — Channel name
 * @param description (optional) — Channel description
 * @param membershipType (optional) — Channel type
 * @returns channel
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftTeamsCreateChannel({ provider: Struct, teamId: string, displayName: string, description?: string, membershipType?: string }): { channel: Struct, errorMessage: string };

/**
 * Create a new Microsoft Team
 * @param provider — Microsoft Graph provider
 * @param displayName — Team name
 * @param description (optional) — Team description
 * @param visibility (optional) — Team visibility
 * @returns teamId
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftTeamsCreateTeam({ provider: Struct, displayName: string, description?: string, visibility?: string }): { teamId: string, errorMessage: string };

/**
 * Get messages from a Microsoft Teams channel
 * @param provider — Microsoft Graph provider
 * @param teamId — ID of the team
 * @param channelId — ID of the channel
 * @param top (optional) — Number of messages to retrieve
 * @returns messages
 * @returns count
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftTeamsGetMessages({ provider: Struct, teamId: string, channelId: string, top?: int }): { messages: Struct[], count: int, errorMessage: string };

/**
 * List all channels in a Microsoft Team
 * @param provider — Microsoft Graph provider
 * @param teamId — ID of the team
 * @returns channels
 * @returns count
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftTeamsListChannels({ provider: Struct, teamId: string }): { channels: Struct[], count: int, errorMessage: string };

/**
 * List all Microsoft Teams the user has joined
 * @param provider — Microsoft Graph provider
 * @returns teams
 * @returns count
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftTeamsListJoined({ provider: Struct }): { teams: Struct[], count: int, errorMessage: string };

/**
 * Send a message to a Microsoft Teams channel
 * @param provider — Microsoft Graph provider
 * @param teamId — ID of the team
 * @param channelId — ID of the channel
 * @param message — Message content
 * @param contentType (optional) — Message content type
 * @returns messageId
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftTeamsSendMessage({ provider: Struct, teamId: string, channelId: string, message: string, contentType?: string }): { messageId: string, errorMessage: string };


// === Data/Microsoft/To Do ===

/**
 * Mark a task as completed in Microsoft To Do
 * @param provider — Microsoft Graph provider
 * @param listId — ID of the task list
 * @param taskId — ID of the task
 * @returns task
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftTodoCompleteTask({ provider: Struct, listId: string, taskId: string }): { task: Struct, errorMessage: string };

/**
 * Create a new Microsoft To Do task list
 * @param provider — Microsoft Graph provider
 * @param displayName — Name of the task list
 * @returns taskList
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftTodoCreateList({ provider: Struct, displayName: string }): { taskList: Struct, errorMessage: string };

/**
 * Create a new task in a Microsoft To Do task list
 * @param provider — Microsoft Graph provider
 * @param listId — ID of the task list
 * @param title — Task title
 * @param body (optional) — Task description
 * @param importance (optional) — Task importance
 * @param dueDate (optional) — Due date (YYYY-MM-DD format)
 * @returns task
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftTodoCreateTask({ provider: Struct, listId: string, title: string, body?: string, importance?: string, dueDate?: string }): { task: Struct, errorMessage: string };

/**
 * Delete a task from Microsoft To Do
 * @param provider — Microsoft Graph provider
 * @param listId — ID of the task list
 * @param taskId — ID of the task
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftTodoDeleteTask({ provider: Struct, listId: string, taskId: string }): string;

/**
 * List all Microsoft To Do task lists
 * @param provider — Microsoft Graph provider
 * @returns taskLists
 * @returns count
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftTodoListLists({ provider: Struct }): { taskLists: Struct[], count: int, errorMessage: string };

/**
 * List all tasks in a Microsoft To Do task list
 * @param provider — Microsoft Graph provider
 * @param listId — ID of the task list
 * @returns tasks
 * @returns count
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftTodoListTasks({ provider: Struct, listId: string }): { tasks: Struct[], count: int, errorMessage: string };

/**
 * Update an existing task in Microsoft To Do
 * @param provider — Microsoft Graph provider
 * @param listId — ID of the task list
 * @param taskId — ID of the task
 * @param title (optional) — New task title (leave empty to keep)
 * @param status (optional) — Task status
 * @param importance (optional) — Task importance
 * @returns task
 * @returns errorMessage
 * @impure has side effects / drives control flow
 */
declare function dataMicrosoftTodoUpdateTask({ provider: Struct, listId: string, taskId: string, title?: string, status?: string, importance?: string }): { task: Struct, errorMessage: string };


// === Data/Notion ===

/**
 * Appends child blocks to a Notion block or page
 * @param provider — Notion provider (from Notion node)
 * @param blockId — The block or page ID to append children to
 * @param children (optional) — Array of Notion block objects to append
 * @param afterBlockId (optional) — Optional sibling block ID to insert after
 * @returns blocks — Appended blocks
 * @returns count — Number of appended blocks
 * @impure has side effects / drives control flow
 */
declare function dataNotionAppendBlockChildren({ provider: Struct, blockId: string, children?: Struct[], afterBlockId?: string }): { blocks: Struct[], count: int };

/**
 * Creates a Notion data source inside an existing database
 * @param provider — Notion provider (from Notion node)
 * @param databaseId — Parent Notion database ID
 * @param title — Data source title
 * @param properties (optional) — Data source property schema in Notion API format
 * @param iconEmoji (optional) — Optional emoji icon
 * @returns dataSource — Created data source
 * @returns dataSourceId — Created data source ID
 * @impure has side effects / drives control flow
 */
declare function dataNotionCreateDataSource({ provider: Struct, databaseId: string, title: string, properties?: Struct, iconEmoji?: string }): { dataSource: Struct, dataSourceId: string };

/**
 * Creates a new page under a Notion data source, database, or page
 * @param provider — Notion provider (from Notion node)
 * @param databaseId — The data source, database, or page ID to create the page under
 * @param parentType (optional) — Parent type for the page
 * @param properties (optional) — Page properties in Notion API format
 * @param content (optional) — Optional page content as an array of Notion block objects
 * @param iconEmoji (optional) — Optional: Emoji to use as the page icon
 * @returns page — The created page info
 * @returns pageId — The ID of the created page
 * @returns pageUrl — The URL of the created page
 * @impure has side effects / drives control flow
 */
declare function dataNotionCreatePage({ provider: Struct, databaseId: string, parentType?: string, properties?: Struct, content?: Struct[], iconEmoji?: string }): { page: Struct, pageId: string, pageUrl: string };

/**
 * Moves a Notion block to trash
 * @param provider — Notion provider (from Notion node)
 * @param blockId — The Notion block ID to delete
 * @returns block — Deleted block
 * @impure has side effects / drives control flow
 */
declare function dataNotionDeleteBlock({ provider: Struct, blockId: string }): Struct;

/**
 * Downloads a Notion file URL into a FlowPath
 * @param fileUrl (optional) — Signed Notion file URL. If empty, File Object is used.
 * @param fileObject (optional) — Notion file object containing file.url, external.url, or url
 * @param outputPath — FlowPath to write the downloaded file into
 * @returns path — Written FlowPath
 * @returns size — Downloaded file size in bytes
 * @returns contentType — Response content type
 * @impure has side effects / drives control flow
 */
declare function dataNotionDownloadFile({ fileUrl?: string, fileObject?: Struct, outputPath: Struct }): { path: Struct, size: int, contentType: string };

/**
 * Retrieves a Notion data source schema with its properties
 * @param provider — Notion provider (from Notion node)
 * @param dataSourceId — The ID of the Notion data source to retrieve
 * @returns dataSource — The Notion data source
 * @returns title — Data source title
 * @returns propertyNames — List of property names in the data source
 * @impure has side effects / drives control flow
 */
declare function dataNotionGetDataSource({ provider: Struct, dataSourceId: string }): { dataSource: Struct, title: string, propertyNames: string[] };

/**
 * Retrieves a Notion database schema with its properties
 * @param provider — Notion provider (from Notion node)
 * @param databaseId — The ID of the Notion database to retrieve
 * @returns database — The Notion database schema
 * @returns title — The database title
 * @returns propertyNames — List of property names in the database
 * @returns dataSourceIds — List of data source IDs belonging to this database
 * @impure has side effects / drives control flow
 */
declare function dataNotionGetDatabase({ provider: Struct, databaseId: string }): { database: Struct, title: string, propertyNames: string[], dataSourceIds: string[] };

/**
 * Retrieves a Notion page with its content and blocks
 * @param provider — Notion provider (from Notion node)
 * @param pageId — The ID of the Notion page to retrieve
 * @param includeContent (optional) — Whether to fetch the page content blocks
 * @param includeNestedContent (optional) — Whether to fetch child blocks recursively
 * @returns page — The Notion page with content
 * @returns title — The page title
 * @returns plainText — The page content as plain text
 * @returns blocks — Array of content blocks
 * @impure has side effects / drives control flow
 */
declare function dataNotionGetPage({ provider: Struct, pageId: string, includeContent?: bool, includeNestedContent?: bool }): { page: Struct, title: string, plainText: string, blocks: Struct[] };

/**
 * Lists child blocks for a Notion block or page
 * @param provider — Notion provider (from Notion node)
 * @param blockId — The block or page ID whose children should be listed
 * @param pageSize (optional) — Maximum number of blocks per page (1-100)
 * @param startCursor (optional) — Pagination cursor from a previous list call
 * @param fetchAll (optional) — Fetch every available page of child blocks
 * @returns blocks — Array of child blocks
 * @returns count — Number of blocks
 * @returns hasMore — Whether there are more child blocks
 * @returns nextCursor — Cursor to request the next page
 * @impure has side effects / drives control flow
 */
declare function dataNotionListBlockChildren({ provider: Struct, blockId: string, pageSize?: int, startCursor?: string, fetchAll?: bool }): { blocks: Struct[], count: int, hasMore: bool, nextCursor: string };

/**
 * Lists all databases the integration has access to
 * @param provider — Notion provider (from Notion node)
 * @param query (optional) — Optional search query to filter databases by title
 * @param pageSize (optional) — Maximum number of databases to return (1-100)
 * @param startCursor (optional) — Pagination cursor from a previous list call
 * @param fetchAll (optional) — Fetch every available page of database results
 * @returns databases — Array of Notion databases
 * @returns count — Number of databases returned
 * @returns hasMore — Whether there are more databases available
 * @returns nextCursor — Cursor to request the next page of databases
 * @impure has side effects / drives control flow
 */
declare function dataNotionListDatabases({ provider: Struct, query?: string, pageSize?: int, startCursor?: string, fetchAll?: bool }): { databases: Struct[], count: int, hasMore: bool, nextCursor: string };

/**
 * Connect to Notion using an Internal Integration token. Create an integration at notion.so/my-integrations and paste the token here.
 * @param integrationToken — Your Notion Internal Integration token (starts with 'secret_'). Get it from notion.so/my-integrations
 * @returns provider — Notion provider with authentication token
 */
declare function dataNotionProviderApiKey({ integrationToken: string }): Struct;

/**
 * Connect to Notion using OAuth. Requires OAuth provider configuration in flow-like.config.json.
 * @returns provider — Notion provider with authentication token
 */
declare function dataNotionProviderOauth(): Struct;

/**
 * Queries a Notion data source and returns matching pages or child data sources
 * @param provider — Notion provider (from Notion node)
 * @param dataSourceId — The ID of the Notion data source to query
 * @param filter (optional) — Optional Notion filter object
 * @param sorts (optional) — Optional Notion sorts array
 * @param pageSize (optional) — Maximum number of results per page (1-100)
 * @param startCursor (optional) — Pagination cursor from a previous query
 * @param fetchAll (optional) — Fetch every available page of query results
 * @returns results — Array of query results
 * @returns count — Number of results
 * @returns hasMore — Whether there are more results
 * @returns nextCursor — Cursor to request the next page
 * @impure has side effects / drives control flow
 */
declare function dataNotionQueryDataSource({ provider: Struct, dataSourceId: string, filter?: Struct, sorts?: Struct[], pageSize?: int, startCursor?: string, fetchAll?: bool }): { results: Struct[], count: int, hasMore: bool, nextCursor: string };

/**
 * Queries a Notion database and returns matching pages
 * @param provider — Notion provider (from Notion node)
 * @param databaseId — The ID of the Notion database to query
 * @param filter (optional) — Optional filter object in Notion filter format
 * @param sorts (optional) — Optional Notion sorts array. Overrides Sort Property when provided.
 * @param sortProperty (optional) — Property name to sort by
 * @param sortDirection (optional) — Sort direction (ascending or descending)
 * @param pageSize (optional) — Maximum number of results to return (1-100)
 * @param startCursor (optional) — Pagination cursor from a previous query
 * @param fetchAll (optional) — Fetch every available page of database results
 * @returns pages — Array of Notion pages matching the query
 * @returns count — Number of pages returned
 * @returns hasMore — Whether there are more results available
 * @returns nextCursor — Cursor to request the next page of pages
 * @impure has side effects / drives control flow
 */
declare function dataNotionQueryDatabase({ provider: Struct, databaseId: string, filter?: Struct, sorts?: Struct[], sortProperty?: string, sortDirection?: string, pageSize?: int, startCursor?: string, fetchAll?: bool }): { pages: Struct[], count: int, hasMore: bool, nextCursor: string };

/**
 * Searches across all pages and databases the integration has access to
 * @param provider — Notion provider (from Notion node)
 * @param query — Search query text
 * @param filterType (optional) — Filter results by type: all, page, or database
 * @param sortDirection (optional) — Sort by last edited time
 * @param pageSize (optional) — Maximum number of results to return (1-100)
 * @param startCursor (optional) — Pagination cursor from a previous search
 * @param fetchAll (optional) — Fetch every available page of results
 * @returns results — Array of search results
 * @returns count — Number of results returned
 * @returns hasMore — Whether there are more results available
 * @returns nextCursor — Cursor to request the next page of results
 * @impure has side effects / drives control flow
 */
declare function dataNotionSearch({ provider: Struct, query: string, filterType?: string, sortDirection?: string, pageSize?: int, startCursor?: string, fetchAll?: bool }): { results: Struct[], count: int, hasMore: bool, nextCursor: string };

/**
 * Updates a Notion block with a raw Notion block update object
 * @param provider — Notion provider (from Notion node)
 * @param blockId — The Notion block ID to update
 * @param blockUpdate (optional) — Notion block update object
 * @param inTrash (optional) — Target trash state. True moves the block to trash; false restores when Change Trash State is enabled.
 * @param changeTrashState (optional) — Enable to apply the In Trash value
 * @returns block — Updated block
 * @impure has side effects / drives control flow
 */
declare function dataNotionUpdateBlock({ provider: Struct, blockId: string, blockUpdate?: Struct, inTrash?: bool, changeTrashState?: bool }): Struct;

/**
 * Updates a Notion data source title, description, icon, or property schema
 * @param provider — Notion provider (from Notion node)
 * @param dataSourceId — The Notion data source ID to update
 * @param title (optional) — New title
 * @param description (optional) — New description
 * @param properties (optional) — Property schema updates in Notion API format
 * @param iconEmoji (optional) — Optional new emoji icon
 * @returns dataSource — Updated data source
 * @impure has side effects / drives control flow
 */
declare function dataNotionUpdateDataSource({ provider: Struct, dataSourceId: string, title?: string, description?: string, properties?: Struct, iconEmoji?: string }): Struct;

/**
 * Updates properties of an existing Notion page
 * @param provider — Notion provider (from Notion node)
 * @param pageId — The ID of the page to update
 * @param properties (optional) — Page properties to update in Notion API format
 * @param iconEmoji (optional) — Optional: New emoji to use as the page icon (empty to keep current)
 * @param archived (optional) — Target trash state. True moves the page to trash; false restores it when Change Trash State is enabled.
 * @param changeArchiveState (optional) — Enable to apply the In Trash value. True is still applied automatically for backward compatibility.
 * @returns page — The updated page info
 * @impure has side effects / drives control flow
 */
declare function dataNotionUpdatePage({ provider: Struct, pageId: string, properties?: Struct, iconEmoji?: string, archived?: bool, changeArchiveState?: bool }): Struct;

/**
 * Uploads a FlowPath file to Notion and returns a file_upload object
 * @param provider — Notion provider (from Notion node)
 * @param file — FlowPath file to upload
 * @param filename (optional) — Notion filename. Uses the FlowPath filename when empty.
 * @param contentType (optional) — MIME type. Inferred from the filename when empty.
 * @returns fileUpload — Notion file_upload object
 * @returns fileUploadId — Notion file upload ID
 * @returns fileObject — Notion property/block file object referencing this upload
 * @returns size — Uploaded file size in bytes
 * @impure has side effects / drives control flow
 */
declare function dataNotionUploadFile({ provider: Struct, file: Struct, filename?: string, contentType?: string }): { fileUpload: Struct, fileUploadId: string, fileObject: Struct, size: int };


// === Data/Providers ===

/**
 * Build an AWS credential struct. Supports explicit access keys, named profiles, EC2 instance metadata, EKS web identity (IRSA), STS AssumeRole and the default environment chain. Emits an AwsProvider that any AWS-aware node (S3, Athena, Bedrock, ...) can consume.
 * @param authMode (optional) — How to authenticate: 'access_key' (static keys), 'environment' (default chain: env vars / shared config), 'profile' (~/.aws/credentials profile), 'instance_metadata' (EC2 IMDS), 'web_identity' (EKS IRSA / OIDC token file), 'assume_role' (STS AssumeRole)
 * @param region (optional) — AWS region (e.g. 'us-east-1', 'eu-west-1')
 * @param endpointUrl (optional) — Override endpoint URL for S3-compatible services (LocalStack, MinIO, Cloudflare R2, ...). Leave empty for real AWS.
 * @param accessKeyId (optional) — AWS access key ID (used when auth_mode is 'access_key')
 * @param secretAccessKey (optional) — AWS secret access key (used when auth_mode is 'access_key')
 * @param sessionToken (optional) — Optional STS session token (used when auth_mode is 'access_key')
 * @returns provider — AWS provider with authentication
 * @impure has side effects / drives control flow
 */
declare function dataAwsProvider({ authMode?: string, region?: string, endpointUrl?: string, accessKeyId?: string, secretAccessKey?: string, sessionToken?: string }): Struct;

/**
 * Build an Azure credential struct. Supports storage account key, SAS token, full connection string, service-principal (tenant/client/secret), managed identity, workload identity and Azure CLI cached tokens. Emits an AzureProvider that any Azure-aware node (Blob, ADLS, ...) can consume.
 * @param authMode (optional) — How to authenticate: 'account_key' (storage key), 'sas_token', 'connection_string', 'client_secret' (service principal), 'managed_identity', 'workload_identity' (AKS federated), 'azure_cli' (cached az login), 'oauth' (Entra ID bearer token)
 * @param account (optional) — Azure storage account name (for Blob / ADLS / managed-identity-on-storage flows)
 * @param endpoint (optional) — Override endpoint (Azurite, sovereign clouds). Leave empty for Azure public cloud.
 * @param accessKey (optional) — Storage account key (used when auth_mode is 'account_key')
 * @returns provider — Azure provider with authentication
 * @impure has side effects / drives control flow
 */
declare function dataAzureProvider({ authMode?: string, account?: string, endpoint?: string, accessKey?: string }): Struct;

/**
 * Build a Cloudflare credential struct. Supports scoped API tokens, legacy email + global API key, R2 S3-compatible access keys and Origin CA keys. Emits a CloudflareProvider that CF-aware nodes (R2 stores, DNS API, Workers, ...) can consume.
 * @param authMode (optional) — How to authenticate: 'api_token' (scoped, preferred), 'global_api_key' (legacy email+key), 'r2' (S3-compatible R2 keys), 'origin_ca_key' (Origin CA)
 * @param accountId (optional) — Cloudflare account ID (required for R2 and some APIs)
 * @param apiToken (optional) — Scoped Cloudflare API token (dash.cloudflare.com/profile/api-tokens)
 * @returns provider — Cloudflare provider with authentication
 * @impure has side effects / drives control flow
 */
declare function dataCloudflareProvider({ authMode?: string, accountId?: string, apiToken?: string }): Struct;

/**
 * Build a Google Cloud credential struct. Supports application default credentials, service account JSON, service account key file (FlowPath), workload identity and static access tokens. Emits a GcpProvider that any GCP-aware node (BigQuery, GCS, ...) can consume.
 * @param authMode (optional) — How to authenticate: 'application_default' (ADC), 'service_account_json' (raw JSON), 'service_account_file' (FlowPath), 'workload_identity' (GKE/metadata), 'access_token' (static bearer)
 * @param defaultProjectId (optional) — Default GCP project used by consumers that don't override it
 * @param readonly (optional) — Request only read-only scopes when the auth mode supports it
 * @returns provider — GCP provider with authentication
 * @impure has side effects / drives control flow
 */
declare function dataGcpProvider({ authMode?: string, defaultProjectId?: string, readonly?: bool }): Struct;


// === Data/QR ===

/**
 * Encode text as a barcode image
 * @param data — Text to encode
 * @param format (optional) — Barcode Format
 * @param scale (optional) — Pixels per barcode module
 * @param margin (optional) — Quiet zone in modules
 * @returns imageOut — Barcode image
 * @impure has side effects / drives control flow
 */
declare function writeQrcode({ data: string, format?: string, scale?: int, margin?: int }): Struct;


// === Data/TDMS ===

/**
 * Extracts metadata (groups, channels, properties) from a LabVIEW TDMS file.
 * @param tdmsPath — Path to the TDMS file
 * @returns metadata — TDMS file metadata struct
 * @impure has side effects / drives control flow
 */
declare function tdmsMetadata({ tdmsPath: Struct }): Struct;

