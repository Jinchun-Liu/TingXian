export type ConversationScenario = 'general' | 'interview' | 'learning' | 'meeting';
export const scenarioNames:Record<ConversationScenario,string> = {general:'通用对话',interview:'面试辅助',learning:'学习问答',meeting:'会议讨论'};
// Preferences from the previous release have no scenario and retain interview behavior.
export function scenarioOf(preferences?:{scenario?:ConversationScenario}|null):ConversationScenario {
  return preferences ? preferences.scenario || 'interview' : 'general';
}
export function automaticAtStart(scenario:ConversationScenario,allowed:boolean,hasKey:boolean) {
  return scenario==='interview' && allowed && hasKey;
}
export const recentDiscussionRequest='请仅根据当前提供的最近实际发言整理最近讨论摘要，分为已明确的结论、待确认事项和行动项。未指定的负责人或截止日期写“未指定”。不要把AI建议或检索资料当作会上决议，并说明这不是整场会议纪要。';
export const learningExplanationRequest='请解释当前上下文中最近讨论的知识点，先给核心概念，再用一个例子说明，并指出一个常见误区；如果资料不足以判断讨论主题，请先澄清。';
