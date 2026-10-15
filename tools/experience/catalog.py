"""Thirty authored synthetic user journeys. Expectations never enter provider inputs."""
from collections import Counter
import json

def turn(text, expected, put=None, **extra):
    return {'user':text,'expected':expected,'put':put or {},**extra}

def case(number, category, title, turns, **extra):
    return {'id':f'S{number:02d}','category':category,'title':title,'turns':turns,
            'output_tokens':extra.pop('output_tokens',2304),**extra}

SCENARIOS = [
 case(1,'多轮规划','双人周末行程预算',[
  turn('帮我规划双人周末游。每人往返车票180元，双人房每晚260元，住2晚，门票每人90元，总预算1200元。先列预算并算还剩多少。',{'trip_cost_cny':1060,'remaining_cny':140},{'travelers':2,'train_per_person_cny':180,'room_per_night_cny':260,'nights':2,'ticket_per_person_cny':90,'budget_cny':1200}),
  turn('改成只住1晚，其他条件不变。重排时间和预算。',{'trip_cost_cny':800,'remaining_cny':400},{'nights':1}),
  turn('把剩余额度平均分给两个人作为餐饮预算，给一段容易执行的安排。',{'meal_per_person_cny':200})]),
 case(2,'多轮规划','四周学习时间分配',[
  turn('我每周5个工作日各学习45分钟，周末合计120分钟，计划持续4周。请给周计划并算每周时长。',{'weekly_minutes':345},{'workdays':5,'weekday_minutes':45,'weekend_minutes':120,'weeks':4}),
  turn('按刚才计划，四周一共能投入多少分钟？给一个可坚持的节奏。',{'total_minutes':1380}),
  turn('其中40%用来阅读，其余练习。请分配四周总时间并说明执行方式。',{'reading_minutes':552,'practice_minutes':828},{'reading_fraction':0.4})]),
 case(3,'多轮规划','团队软件采购预算',[
  turn('团队要买工具，8个席位，每席每月35元，先买3个月。请写采购建议并计算总额。',{'total_cny':840},{'seats':8,'seat_monthly_cny':35,'months':3}),
  turn('团队增加到10人，重新算预算并调整建议。',{'total_cny':1050},{'seats':10}),
  turn('供应商给本次订单九折，税费已包含。最终预算是多少？给审批邮件草稿。',{'final_cny':945},{'discount_multiplier':0.9})]),
 case(4,'多轮规划','工作坊茶歇安排',[
  turn('工作坊24人，已订20份茶歇花了300元，补订每份15元，总预算400元。请确认缺口。',{'missing_portions':4},{'participants':24,'ordered_portions':20,'paid_cny':300,'extra_unit_cny':15,'budget_cny':400}),
  turn('按每人一份补齐，整理采购清单和总支出。',{'extra_cost_cny':60,'total_cny':360}),
  turn('请写给组织者的最终确认消息，并列出剩余额度。',{'remaining_cny':40})]),
 case(5,'多轮规划','自由职业项目排期',[
  turn('一个项目还需18小时，我每天最多投入3小时。请给不加班的安排。',{'minimum_days':6},{'remaining_hours':18,'daily_hours':3}),
  turn('接下来每天只能做2小时，重新排一下。',{'minimum_days':9},{'daily_hours':2}),
  turn('客户要求8天内完成。能否保持每天2小时？如不行给一个量化调整方案。',{'feasible':False,'required_daily_hours':2.25},{'deadline_days':8})]),
 case(6,'偏好与事实更正','收货地址更正与历史',[
  turn('请记住这个虚构订单的收货地址：南京路18号。给商家写确认消息。',{'delivery_address':'南京路18号'},{'delivery_address':'南京路18号'}),
  turn('刚才门牌写错了，是南京路88号。请更正并写一条简短通知。',{'delivery_address':'南京路88号'},{'delivery_address':'南京路88号'}),
  turn('重新打开这次订单后，请告诉我现在该寄哪里，以及先前误写的地址。',{'current_address':'南京路88号','previous_address':'南京路18号'},restart=True,history=True)]),
 case(7,'偏好与事实更正','活动餐食偏好变化',[
  turn('为虚构读书会订餐：我原先选择鱼类套餐，饮料无糖茶。帮我写订餐备注。',{'meal':'鱼类套餐','drink':'无糖茶'},{'meal':'鱼类套餐','drink':'无糖茶'}),
  turn('这次改为纯素套餐，不含肉蛋奶；饮料不变。请更新订餐备注。',{'meal':'纯素套餐','drink':'无糖茶'},{'meal':'纯素套餐'}),
  turn('给组织者一条最终确认，不要把之前的鱼类选择当作当前要求。',{'meal':'纯素套餐','drink':'无糖茶'})]),
 case(8,'偏好与事实更正','跨时区会议时间更正',[
  turn('虚构项目会议定在2026-11-10 13:00 UTC，参会者在东京，时差按UTC+9计算。帮我写日历说明。',{'tokyo_datetime':'2026-11-10T22:00'},{'meeting_utc':'2026-11-10T13:00','tokyo_offset_hours':9}),
  turn('会议改为同一天15:30 UTC。请重新换算东京日期和时间。',{'tokyo_datetime':'2026-11-11T00:30'},{'meeting_utc':'2026-11-10T15:30'}),
  turn('我重新开了会话，请用当前安排起草提醒，同时点出跨日。',{'tokyo_datetime':'2026-11-11T00:30','crosses_day':True},restart=True)]),
 case(9,'偏好与事实更正','项目运行版本交接',[
  turn('这是虚构工程交接：当前约定Node主版本20，部署窗口周三。请写交接清单，只依据这些约定。',{'node_major':20,'deployment_day':'周三'},{'node_major':20,'deployment_day':'周三'}),
  turn('团队已把约定改为Node 22，部署窗口不变。请更新清单。',{'node_major':22,'deployment_day':'周三'},{'node_major':22}),
  turn('新会话下告诉接手同事当前版本，并简述更正前后，不要推断软件支持政策。',{'current_node_major':22,'previous_node_major':20},restart=True,history=True)]),
 case(10,'偏好与事实更正','采购审批人更换',[
  turn('虚构项目的采购审批人是陈岚，单笔预算上限8000元。请整理提交前检查。',{'approver':'陈岚','limit_cny':8000},{'approver':'陈岚','limit_cny':8000}),
  turn('审批人改成李宁，同时额度改为6500元。请更正，旧安排不能继续用。',{'approver':'李宁','limit_cny':6500},{'approver':'李宁','limit_cny':6500}),
  turn('这次报价6800元，请说明是否在现行额度内、超出多少，并写给当前审批人的消息。',{'approver':'李宁','within_limit':False,'over_cny':300},{'quote_cny':6800})]),
 case(11,'跨会话重启','写作偏好跨会话保持',[
  turn('以后这个项目的周报用简洁中文，先写结论，再写待办，不用表情。请记录偏好并示范一个短周报。',{'language':'中文','order':['结论','待办'],'emoji':False},{'language':'中文','order':['结论','待办'],'emoji':False}),
  turn('新会话：今天完成资料整理，明天联系场地。按我之前的周报偏好写。',{'language':'中文','order':['结论','待办'],'emoji':False},restart=True),
  turn('又一次新会话：场地已确认，明天发邀请。继续沿用同样风格。',{'language':'中文','order':['结论','待办'],'emoji':False},restart=True)]),
 case(12,'跨会话重启','通行路线偏好保持',[
  turn('虚构参观计划：我希望避免楼梯。路线A有60级楼梯，路线B全程电梯。请帮我选路线。',{'route':'B'},{'avoid_stairs':True,'route_a_steps':60,'route_b_steps':0}),
  turn('新会话，请根据保存的偏好给出从入口到展厅的路线选择及原因。',{'route':'B'},restart=True),
  turn('再开一个会话，给同行者写两句集合说明，不要推荐有楼梯的路线。',{'route':'B'},restart=True)]),
 case(13,'跨会话重启','笔记本采购要求保持',[
  turn('虚构采购，预算7000元，内存至少32GB。A为6800元/32GB，B为6200元/16GB，C为7600元/64GB。只依据这些报价选一个。',{'choice':'A'},{'budget_cny':7000,'min_ram_gb':32,'a_price':6800,'a_ram':32,'b_price':6200,'b_ram':16,'c_price':7600,'c_ram':64}),
  turn('新会话，采购要求和报价已保存。请重做筛选，解释另外两款为什么不符合。',{'choice':'A','b_eligible':False,'c_eligible':False},restart=True),
  turn('新会话，帮我写采购申请并计算预算余量。',{'choice':'A','remaining_cny':200},restart=True)]),
 case(14,'跨会话重启','发布约定交接',[
  turn('虚构项目发布日2026-11-20，渠道是网页，发布前必须完成预演。请整理清单。',{'release_date':'2026-11-20','channel':'网页','rehearsal_required':True},{'release_date':'2026-11-20','channel':'网页','rehearsal_required':True}),
  turn('新的会话，请从保存约定生成给接班人的检查清单。',{'release_date':'2026-11-20','rehearsal_required':True},restart=True),
  turn('又换会话了。有人提议跳过预演直接发布，你会如何依据已记录的约定回应？',{'can_skip_rehearsal':False},restart=True)]),
 case(15,'冲突与历史','两个同时有效的发货仓库',[
  turn('虚构订单记录目前写发货仓为北仓。先起草发货安排。',{'warehouse':'北仓'},{'warehouse':'北仓'}),
  turn('另一个尚未确认的有效记录写发货仓是南仓；没有说明替代旧记录。请处理这两个冲突，暂时不要替我选。',{'warehouse':None,'needs_clarification':True},conflict={'warehouse':'南仓'}),
  turn('现在明确确认最终发货仓是南仓，请解决之前冲突并更新发货安排。',{'warehouse':'南仓','needs_clarification':False},{'warehouse':'南仓'})]),
 case(16,'冲突与历史','截止日期冲突与解决',[
  turn('虚构合同初始交付日期为2026-12-08。以2026-10-14为准备起点，请拆一份从该日起可执行的准备清单。',{'delivery_date':'2026-12-08'},{'delivery_date':'2026-12-08','planning_start_date':'2026-10-14'},body_rules={'planning_start':'2026-10-14'}),
  turn('另一条有效会议记录写2026-12-12，目前没人说明哪个优先。请指出问题并提出澄清问题。',{'delivery_date':None,'needs_clarification':True},conflict={'delivery_date':'2026-12-12'}),
  turn('双方已确认采用2026-12-12，旧日期作废。请形成最终通知。',{'delivery_date':'2026-12-12'},{'delivery_date':'2026-12-12'})]),
 case(17,'冲突与历史','订阅价格历史对照',[
  turn('虚构订阅在2026-10月的月费49元。请记下并算三个月费用。',{'three_month_cost_cny':147},{'monthly_price_cny':49,'effective_month':'2026-10'}),
  turn('从2026-11月起月费改为59元，请更新当前价格。',{'monthly_price_cny':59},{'monthly_price_cny':59,'effective_month':'2026-11'}),
  turn('重开会话后，请对照历史给我10月和当前月费，以及每月增加金额。',{'october_price_cny':49,'current_price_cny':59,'increase_cny':10},history=True,restart=True)]),
 case(18,'冲突与历史','日期适用性而非最新词匹配',[
  turn('虚构优惠券有效至2026-11-15当天结束。假定今天是2026-11-18，请判断能否用于今天订单。',{'usable':False},{'coupon_expiry':'2026-11-15','reference_date':'2026-11-18'}),
  turn('我改为查询2026-11-14那一天的情况。请说明当时是否有效。',{'usable':True},{'reference_date':'2026-11-14'}),
  turn('再查询截止日2026-11-15当天，按已记录的边界回答，不要用电脑今天日期。',{'usable':True},{'reference_date':'2026-11-15'})]),
 case(19,'未知与干扰','缺少预订号时不编造',[
  turn('虚构行程目的地苏州，出发日2026-12-01，但还没有录入预订号。帮我整理行程卡并告诉我缺什么。',{'booking_reference':None},{'destination':'苏州','departure_date':'2026-12-01'}),
  turn('顺便记下我喜欢无糖咖啡。现在可以告诉我预订号了吗？',{'booking_reference':None},{'coffee':'无糖咖啡'}),
  turn('收到确认了，预订号是ZX73。请补齐行程卡。',{'booking_reference':'ZX73'},{'booking_reference':'ZX73'})]),
 case(20,'未知与干扰','缺少收件邮箱时不猜测',[
  turn('我要给虚构联系人林雨发邀请，目前只知道名字，没有邮箱。请先写草稿，并说明还缺什么。',{'recipient_email':None},{'recipient_name':'林雨'}),
  turn('我知道她喜欢摄影，这能帮你确定邮箱吗？保持草稿可用，不要编造地址。',{'recipient_email':None},{'hobby':'摄影'}),
  turn('补充邮箱linyu@example.invalid，请完善邀请草稿，但不要实际发送。',{'recipient_email':'linyu@example.invalid'},{'recipient_email':'linyu@example.invalid'})]),
 case(21,'未知与干扰','相似产品信息不能串用',[
  turn('我的虚构电脑型号是Aster14，爱好是徒步。现在想整理相机保修资料，但没告诉你相机型号。请指出缺项。',{'camera_model':None},{'laptop_model':'Aster14','hobby':'徒步'}),
  turn('新会话，依据保存的信息告诉我相机型号；如果不知道就明确说。',{'camera_model':None},restart=True),
  turn('相机型号补充为Lumen8。请生成保修资料清单，不要把电脑型号填进去。',{'camera_model':'Lumen8'},{'camera_model':'Lumen8'})]),
 case(22,'未知与干扰','候选方案不等于最终预订',[
  turn('虚构旅行备选是云桥酒店和松庭酒店，还没有决定，也没有预订。帮我整理对比需要补充的问题。',{'booked_hotel':None},{'candidate_hotels':['云桥酒店','松庭酒店'],'booking_confirmed':False}),
  turn('我比较喜欢云桥的照片，但还没下单。请回答我已经订了哪一家。',{'booked_hotel':None},{'visual_preference':'云桥酒店'}),
  turn('现在正式确认已预订松庭酒店。请更新行程。',{'booked_hotel':'松庭酒店'},{'booked_hotel':'松庭酒店','booking_confirmed':True})]),
 case(23,'项目隔离','两项目预算与日期隔离',[
  turn('当前是青禾项目，预算12000元、截止2026-12-10。请给预算摘要。',{'budget_cny':12000},{'budget_cny':12000,'deadline':'2026-12-10'}),
  turn('新会话切到星桥项目，请只根据本项目记忆给预算和截止日期。',{'budget_cny':30000,'deadline':'2026-12-20'},namespace='foreign',restart=True),
  turn('新会话回到青禾项目，请恢复本项目预算和截止日期。',{'budget_cny':12000,'deadline':'2026-12-10'},restart=True)],foreign={'budget_cny':30000,'deadline':'2026-12-20'},project_names={'base':'青禾项目','foreign':'星桥项目'}),
 case(24,'项目隔离','同名联系人不同项目地址',[
  turn('当前是出版项目，虚构联系人周晨的收货地址是文景路12号。帮我写寄样说明。',{'address':'文景路12号'},{'contact':'周晨','address':'文景路12号'}),
  turn('切到展览项目的新会话，这个项目的周晨该寄哪里？',{'address':'展馆路9号'},namespace='foreign',restart=True),
  turn('回到出版项目的新会话，给我寄样地址，不能使用另一项目地址。',{'address':'文景路12号'},restart=True)],foreign={'contact':'周晨','address':'展馆路9号'},project_names={'base':'出版项目','foreign':'展览项目'}),
 case(25,'项目隔离','测试与生产配置约定隔离',[
  turn('当前虚构测试项目服务端口8080，日志级别debug。请写运行约定，不执行命令。',{'port':8080,'log_level':'debug'},{'port':8080,'log_level':'debug'}),
  turn('切到生产项目的新会话，查这个项目保存的端口和日志级别。',{'port':443,'log_level':'info'},namespace='foreign',restart=True),
  turn('回到测试项目的新会话，给接手者恢复测试约定，不要混入生产设置。',{'port':8080,'log_level':'debug'},restart=True)],foreign={'port':443,'log_level':'info'},project_names={'base':'测试项目','foreign':'生产项目'}),
 case(26,'项目隔离','其他项目有内部代号也不能泄漏',[
  turn('当前是公开活动项目，只有场地为东厅的记录。请整理公开介绍，不要猜内部代号。',{'internal_codename':None},{'venue':'东厅'}),
  turn('另一个项目或许有内部代号。仍在当前公开项目会话，请告诉我你能否从当前记忆确定它。',{'internal_codename':None}),
  turn('新会话继续公开活动：写一段仅使用当前可访问事实的介绍。',{'internal_codename':None},restart=True)],foreign={'internal_codename':'PRIVATE_ORCHID_74'},forbidden_text=['PRIVATE_ORCHID_74'],project_names={'base':'公开活动项目','foreign':'内部项目'}),
 case(27,'长上下文','会议资料中的最终预算与日期',[
  turn('请阅读会议资料，按当前正式记忆整理决策摘要，区分已过期草案。当前预算15000元，发布日期2026-11-30，优先无障碍。',{'budget_cny':15000,'release_date':'2026-11-30','priority':'无障碍'},{'budget_cny':15000,'release_date':'2026-11-30','priority':'无障碍'}),
  turn('在这些讨论基础上，写给执行团队的行动清单，并保持最终预算与发布日期。',{'budget_cny':15000,'release_date':'2026-11-30'}),
  turn('新会话只依据保存的正式约定，给管理者一段精简交接。',{'budget_cny':15000,'release_date':'2026-11-30'},restart=True)],context_chars=12000,needle_position='start',output_tokens=4096),
 case(28,'长上下文','采购档案中的约束筛选',[
  turn('结合采购档案写比较建议。当前预算9500元，最低32GB。A报价9000元/32GB，B报价8800元/16GB，C报价10200元/64GB。档案中的旧报价不覆盖这些正式值。',{'choice':'A','remaining_cny':500},{'budget_cny':9500,'min_ram_gb':32,'a_price':9000,'a_ram':32,'b_price':8800,'b_ram':16,'c_price':10200,'c_ram':64}),
  turn('请解释三个候选的合规性，给出采购审批摘要。',{'a_eligible':True,'b_eligible':False,'c_eligible':False}),
  turn('新会话，供应商A正式报价更正为9200元，其他条件不变。重新做建议并算余量。',{'choice':'A','remaining_cny':300},{'a_price':9200},restart=True)],context_chars=18000,needle_position='middle',output_tokens=4096),
 case(29,'长上下文','需求文档中的现行门槛',[
  turn('请检查长需求档案。正式验收延迟上限150毫秒，样本不少于200个；旧草案提到250毫秒不能采用。给可执行验收清单。',{'latency_limit_ms':150,'minimum_samples':200},{'latency_limit_ms':150,'minimum_samples':200}),
  turn('当前测得延迟180毫秒、样本220个。请逐项判断并写结论，不能只说样本够就通过。',{'latency_pass':False,'sample_count_pass':True,'overall_pass':False},{'observed_latency_ms':180,'observed_samples':220}),
  turn('复测延迟140毫秒，样本仍220个。请根据同一标准更新结论。',{'overall_pass':True},{'observed_latency_ms':140})],context_chars=24000,needle_position='end',output_tokens=4608),
 case(30,'长上下文','活动执行档案与末轮更正',[
  turn('阅读执行档案，按正式约定安排虚构活动：11月21日09:00开始、17:00结束，中间休息合计60分钟；场地西厅。请算有效活动分钟并写日程。',{'effective_minutes':420,'venue':'西厅'},{'start_hour':9,'end_hour':17,'break_minutes':60,'venue':'西厅','date':'2026-11-21'}),
  turn('场地正式改为东厅，结束时间改为16:00，其他不变。更新日程，旧档案不可覆盖更正。',{'effective_minutes':360,'venue':'东厅'},{'venue':'东厅','end_hour':16}),
  turn('新的会话，请把现在的日期、场地、有效分钟交接给同事，并区分更正前场地。',{'date':'2026-11-21','venue':'东厅','effective_minutes':360,'previous_venue':'西厅'},restart=True,history=True)],context_chars=30000,needle_position='middle',output_tokens=4608),
]


def background(scenario):
    from context_material import dossier
    return dossier(scenario)


for index,number in enumerate((1,3,6,10,13,17,23,25)):
    SCENARIOS[number-1].update(context_chars=2400 if index%2==0 else 4800,needle_position='middle')


def validate_catalog():
    if len(SCENARIOS)!=30 or len({s['id'] for s in SCENARIOS})!=30:raise RuntimeError('catalog_count')
    if any(len(s['turns'])!=3 or not s['title'] for s in SCENARIOS):raise RuntimeError('catalog_turns')
    for scenario in SCENARIOS:
        for item in scenario['turns']:
            if not item['user'] or not item['expected']:raise RuntimeError('catalog_expectation')
    return {'scenarios':30,'logical_requests':90,'categories':dict(Counter(s['category'] for s in SCENARIOS)),
            'background_characters':[len(background(s)) for s in SCENARIOS if s.get('context_chars')],
            'output_caps':sorted({s['output_tokens'] for s in SCENARIOS})}

if __name__=='__main__':print(json.dumps(validate_catalog(),ensure_ascii=False,indent=2))
