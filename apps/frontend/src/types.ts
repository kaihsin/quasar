export type WorkSource = "github" | "jira";

export interface SourceWarning {
  source: WorkSource;
  message: string;
}

export interface WorkItem {
  source: WorkSource;
  id: string;
  external_id: string;
  title: string;
  url: string;
  status: string;
  assignees: string[];
  labels: string[];
  priority: string | null;
  created_at: string;
  updated_at: string;
  start_date: string;
  target_date: string;
  author: string | null;
  container: string;
  repo: string | null;
  source_metadata: unknown;
}

export interface WorkItemsResponse {
  data: WorkItem[];
  warnings: SourceWarning[];
  fetched_at: string;
  cache_status: string;
}

export interface Comment {
  author: string | null;
  created_at: string;
  body: string;
}

export interface AssigneeOption {
  id: string;
  name: string;
}

export interface WorkItemDetail {
  item: WorkItem;
  body: string | null;
  comments: Comment[];
  project_status: string | null;
  status_options: string[];
  assignee_options: AssigneeOption[];
  assignee_selected: string[];
}

export type WorkItemFieldKind = "start" | "target" | "status";

export interface PersonWorkItems {
  user: string;
  account_id: string | null;
  created_by: WorkItem[];
  mentioned: WorkItem[];
}
