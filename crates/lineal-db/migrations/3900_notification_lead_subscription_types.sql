-- 3900 (Paket 6, B6.01): Projekt-Leads wurden mit leerer Typenliste abonniert und erhielten daher
-- keine Statusberichte. Bestand auf den Default-Typ des Projekt-Abos (docs/05 §6.3) heben.
update notification_subscription
   set notification_subscription_types = array['projectUpdateCreated']::text[]
 where target_type = 'project'
   and cardinality(notification_subscription_types) = 0;
