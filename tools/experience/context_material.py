"""Generated synthetic dossiers: dated versions, similar entities, rejected and pending alternatives."""
import json
from datetime import date,timedelta

def dossier(scenario):
    target=scenario.get('context_chars',0)
    if not target:return ''
    official=('【初始正式摘要；其后以本轮MCP更正为准】'+scenario['title']+'。'
              +json.dumps(scenario['turns'][0]['put'],ensure_ascii=False)+'。这是源事实，不是计算结果或评分答案。\n')
    peers=['青禾一组','青禾二组','青禾试点组','青禾资料组','星桥一组','星桥二组','星桥试点组','星桥资料组']
    templates=[
      '会议纪要：{entity}讨论{title}，提议在{later}提交候选。记录的测算为{money}元，时间缓冲{minutes}分钟。状态为待审批，未进入当前项目正式记忆。负责人要求先核对范围，再比较报价；本条不能作为最终承诺。',
      '版本差异：文档V{v}把计划日期从{day}暂改为{later}，仅供{entity}内部排练。旧附件仍保留上一版本，评审指出名称相似容易误发。交接人应注明项目、版本和生效范围，不能将此排练时间替换当前用户确认的日期。',
      '候选报价：供应方{entity}提供{money}元的草案，含{count}项服务但不含最终确认项。报价有效期截至{later}；此材料没有签署记录。采购讨论要求把基础报价、附加费用和验收条件拆开，不能仅选数值最低者。',
      '历史回溯：在{day}的早期讨论中，{entity}倾向另一个方案，试算额度{money}元。后来因为范围、版本及交付责任不清被退回。记录保留用于解释演变，不表示恢复旧决定；活动Claim与来源证据仍需逐项核实。',
      '相似实体核对：名单同时出现{entity}和{peer}。前者负责{count}份资料，后者负责{minutes}分钟的说明环节。会议提醒不要仅凭简称合并两人的任务，也不要把其他项目的地址、预算或配置当作当前项目事实。',
      '测试备忘：{entity}建议取{count}个样本，观察窗口{minutes}分钟。这是一份方法讨论，不是已经测得的结果。验收时需要分别报告阈值、观测值和样本范围；某一指标合格不能自动代表其他指标也合格。',
      '风险登记：依赖材料拟于{later}到齐，{entity}尚未确认。若材料延迟，应保留主计划与备选方案的差别，并向负责人提出明确问题。当前没有批准追加{money}元，也没有授权跳过必要检查。',
      '交接邮件摘录：{entity}请{peer}在{day}复核文档V{v}，并保留更正来源。邮件只说明处理流程，未改变正式约定。接班人应区分已完成、待确认和已否决三种状态，未知收件地址或订单编号不可自行补全。',
      '资源表解释：草案为{entity}预留{count}份材料和{minutes}分钟讨论；资源占用尚未确认。若主任务缩减天数、变更人数或更新单价，应从当前事实重算，不要把此前示例总额继续复制到审批消息。',
      '退回意见：文档V{v}混用了{peer}的旧数值和{entity}的现行名称，因此在{day}退回。退回不等于审批通过。修改者必须附来源记录；仅凭相同词语、相似日期或看起来合理的数值不能证明属于当前项目。',
      '边界讨论：{entity}询问截止日{later}是否包含当天，记录员答复需要按正式约定判定。此问答没有自行设定全局时间规则。若用户指定参考日期，应按指定日期评估，不能把运行机器的今天替代历史问题。',
      '收尾记录：{entity}已归档文档V{v}，下一次审阅计划在{later}。归档包包含{count}份附件与未采纳备选，不保证每条内容仍有效。最终摘要应把当前决策、证据、更正前内容及尚未解决的冲突分别说明。',
    ]
    blocks=[];index=0;length=0
    while length<target:
        day=date(2026,8,1)+timedelta(days=index%72)
        values=dict(entity=peers[index%len(peers)],peer=peers[(index+3)%len(peers)],title=scenario['title'],
                    day=day.isoformat(),later=(day+timedelta(days=3+index%17)).isoformat(),v=index//8+1,
                    money=2300+(index*173)%21000,minutes=15+(index*7)%105,count=8+(index*11)%250)
        block=f'【档案{index+1:03d}｜{day.isoformat()}｜背景记录，非本轮确认】'+templates[index%len(templates)].format(**values)+'\n\n'
        blocks.append(block);length+=len(block);index+=1
    text=''.join(blocks)[:target-len(official)]
    position={'start':0,'middle':len(text)//2,'end':len(text)}[scenario.get('needle_position','middle')]
    return text[:position]+official+text[position:]
