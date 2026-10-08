-- Tasks can set and remove their project's secrets now: the starter rule
-- allowing every secrets tool keeps allowing only reading.
update rules set tool = 'secrets.secrets_{list,get}' where tool = 'secrets.*' and command is null and target is null and action = 'allow';
