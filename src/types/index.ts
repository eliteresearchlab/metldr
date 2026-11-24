export interface Theme {
  primary: string
  secondary: string
  accent: string
  bg: string
  bgSecondary: string
  text: string
  textMuted: string
  border: string
  borderSubtle: string
  shadow: string
}

export type ThemeName = 'light' | 'cyberpunk' | 'catppuccin' | 'gruvbox'
export type ThemeListener = (themeName: string, theme: Theme) => void

export interface IntentStyle {
  bg: string
  text: string
}

export interface EmailMetadata {
  from?: string | null
  sender?: string | null
  fromEmail?: string | null
  senderEmail?: string | null
  replyTo?: string | null
  'reply-to'?: string | null
  to?: string | null
  toList?: string[]
  toRecipients?: string[]
  cc?: string | null
  ccList?: string[]
  ccRecipients?: string[]
  bcc?: string | null
  bccList?: string[]
  bccRecipients?: string[]
  date?: string | null
  subject?: string
  mailedBy?: string | null
  signedBy?: string | null
  participants?: string[]
  emailCount?: number
}

export interface ReplySuggestion {
  id: string
  tone: string
  length: string
  body: string
  label: string
}

export interface ContextData {
  contextBefore?: string
  contextAfter?: string
  fullSentence?: string
  pageLang?: string
}

// storage
export interface SupportedLanguage {
  code: string
  name: string
  url: string
}

// ui summary display
export interface Summary {
  summary?: string
  action_items?: string[]
  dates?: string[]
  confidence?: 'high' | 'medium' | 'low'
  model?: string
  intent?: string
  time_ms?: number
  cached?: boolean
}

// inboxsdk types
export interface InboxSDKCompose {
  registerComposeViewHandler: (handler: (composeView: ComposeView) => void) => void
}

export interface InboxSDK {
  Compose: InboxSDKCompose
}

export interface ComposeView {
  isInlineReplyForm?: () => boolean
  getThreadView?: () => ThreadView | null
  getThreadID?: () => string
  getElement?: () => HTMLElement | null
  getToRecipients: () => Contact[]
  getCcRecipients: () => Contact[]
  getBccRecipients: () => Contact[]
  getSubject: () => string
  getTextContent: () => string
  getHTMLContent: () => string
  insertHTMLIntoBodyAtCursor: (html: string) => void
  insertTextIntoBodyAtCursor: (text: string) => void
  setBodyHTML: (html: string) => void
  addButton: (options: { title: string; iconUrl: string; orderHint: number; onClick: (event: Event) => void }) => void
  on: (event: string, callback: () => void) => void
}

export interface ThreadView {
  getThreadIDAsync?: () => Promise<string>
  getSubject: () => string
  getMessageViewsAll: () => MessageView[]
  getElement?: () => HTMLElement | null
}

export interface MessageView {
  getBodyElement: () => HTMLElement | null
  getSender: () => Contact | null
  getRecipientEmailAddresses: () => string[]
  getRecipientsFull: () => Promise<Contact[]>
  getReplyTo?: () => Contact | Contact[] | null
  getSMTPHeaders?: () => Record<string, string> | null
  getDateString?: () => string | null
  isLoaded?: () => boolean
  getViewState?: () => 'EXPANDED' | 'COLLAPSED' | 'HIDDEN'
  getMessageIDAsync?: () => Promise<string>
  on?: (event: string, callback: (e: unknown) => void) => void
}

export interface Contact {
  name?: string
  fullName?: string
  displayName?: string
  emailAddress?: string
  address?: string
  email?: string
}

// email extractor
export type EmailProcessCallback = () => void

export interface SummaryResponse {
  summary?: unknown
  action_items?: string[]
  dates?: string[]
  key_facts?: {
    booking_reference?: string
    amount?: string
    sender_org?: string
  }
  intent?: string
  reasoning?: string
  urgency?: string
  time_ms?: number
  cached?: boolean
  model?: string
  error?: string
}

// reply panel
export interface FetchResponse {
  success: boolean
  error?: string
  replies?: ReplySuggestion[]
  suggestions?: ReplySuggestion[]
  contextInvalidated?: boolean
}
