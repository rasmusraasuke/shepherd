import { execFile } from "node:child_process"

export default {
  id: "shepherd",
  setup: async (api) => {
    const agent = process.env.SHEPHERD_AGENT
    let reported
    const timer = setInterval(() => {
      const route = api.ui.router.current()
      if (!agent || route.type !== "session" || route.sessionID === reported) return
      reported = route.sessionID
      const hook = JSON.stringify({ session_id: reported })
      execFile("shepherd", ["ctl", "agent-session", agent, hook], () => {})
    }, 1000)
    return () => clearInterval(timer)
  },
}
